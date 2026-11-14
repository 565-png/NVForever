//! The camera's bob while walking: a head bob in first person, none in third (owner,
//! 2026-11-07: "we had headbob in third person, but none in first person? it should be
//! the other way around").
//!
//! - **Before**: the first-person camera sat on the arms rig's camera node, which the
//!   game's first-person movement clips never move (`view_sway.rs` module notes), so
//!   only the arms and weapon bobbed. The third-person camera's pivot was the walking
//!   body's head bone, which rises and falls with every stride.
//! - **First person** [design; every constant an engineering choice]: the whole arms rig
//!   (camera node, arms and weapon together) dips at each footfall and swings a little
//!   sideways once a stride, in step with the movement clip the arms rig plays (the
//!   footfall phase of `view_sway`), growing with the ground speed up to a run's, less
//!   sneaking, little down the sights, none in the air. A switch of the settings page
//!   (`head-bob`, `--no-head-bob`), under Gameplay › Inertia.
//! - **Inertia** [design: the owner's playtest of 2026-11-13, "we should add inertia to
//!   it"]: the head rides critically damped springs (`nvf_play::ease::Spring`, exact per
//!   frame): the stride's bob goes through one ([`BOB_SETTLE_SECONDS`]) so it rounds off
//!   rather than following the clip rigidly; the body's acceleration leaves the head a
//!   little behind as it speeds up and lets it lean on as it slows ([`LAG_PEAK_PER_SPEED`]);
//!   the vertical acceleration dips it at a take-off and a landing, more for a harder
//!   landing ([`LAND_PEAK_PER_SPEED`]). Only the camera's place moves: its rotation stays
//!   the mouse's. Every spring's time is × the inertia weight.
//! - **Third person**: the pivot's height above the feet is the head's, smoothed by two
//!   first-order lags of [`PIVOT_SECONDS`] each: a stride's rise and fall (about 1.7 a
//!   second at a run) is cut to about an eighth, and crouching or standing up still
//!   lowers or raises the camera, 90 % of the way in a second.

use bevy::math::{Vec2, Vec3};
use nvf_play::ease::{omega_for, Spring};

/// The ground speed (units per second) at which the bob is full: about a run.
const FULL_SPEED: f32 = 300.0;
/// The stride's bob follows its curve through a spring settling in this long (95%).
pub const BOB_SETTLE_SECONDS: f32 = 0.12;
/// The head's lag from the body's acceleration: the spring rate (1/s), the most offset a
/// sudden change of ground speed would give (units per unit/s) and the most shown (units).
const LAG_OMEGA: f32 = 9.0;
pub const LAG_PEAK_PER_SPEED: f32 = 0.003;
const MAX_LAG: f32 = 1.5;
/// Take-offs and landings: the spring rate (1/s), the most dip for a sudden change of
/// vertical speed (units per unit/s: about 1.2 for a standing jump's 291 units/s), the
/// most shown (units).
const LAND_OMEGA: f32 = 10.0;
pub const LAND_PEAK_PER_SPEED: f32 = 0.004;
const MAX_LAND: f32 = 3.0;
/// The full bob: the dip at each footfall and the sideways swing, units (1.6 and 0.5
/// until the owner's fourth playtest of 2026-11-13: "there is no headbob even with it on -
/// add a slight headbob, smooth"; the log showed a 1.5-unit dip, about 2 cm).
pub const DIP: f32 = 3.0;
pub const SWING: f32 = 1.0;
/// The breathing camera [design: the owner, 2026-11-13, "camera moves on its own to
/// simulate breathing - smooth, slow and only a little"]: a rise and fall of this much
/// (units, about 7 mm) once in this long (seconds), with a twentieth of a unit forward and
/// back, all of it standing still and this share while moving, fading over this long.
pub const BREATH_RISE: f32 = 0.5;
pub const BREATH_SECONDS: f32 = 4.5;
const BREATH_AHEAD: f32 = 0.15;
const BREATH_MOVING: f32 = 0.4;
const BREATH_FADE_SECONDS: f32 = 1.0;
/// Below this ground speed (units/s) the body counts as still for the breath.
const STILL_SPEED: f32 = 20.0;
/// Share of the bob down the sights and sneaking.
const SIGHTS: f32 = 0.25;
const SNEAK: f32 = 0.6;
/// Seconds the bob takes to follow the speed, as a time constant.
const FOLLOW_SECONDS: f32 = 0.2;
/// Each of the third-person pivot's two lags, seconds.
pub const PIVOT_SECONDS: f32 = 0.25;
/// A change of the head's height above the feet over this (units) in one frame is not a
/// stride: the pivot goes there at once [design: a teleport, a load, a ragdoll getting up].
const PIVOT_SNAP: f32 = 40.0;

/// What one frame's first-person bob depends on.
#[derive(Debug, Clone, Copy)]
pub struct BobInput {
    pub dt: f32,
    /// Ground speed, units per second.
    pub speed: f32,
    /// The movement clip's phase in its cycle, 0..1.
    pub phase: f32,
    /// The view's compass heading (clockwise from +y), radians.
    pub heading: f32,
    pub sights: bool,
    pub sneaking: bool,
    pub airborne: bool,
    /// Ground velocity in the view's frame (x right, y ahead), units/s.
    pub velocity: Vec2,
    /// The body's vertical velocity off the ground (up), units/s; 0 on it.
    pub vertical: f32,
    /// The inertia weight: the springs' times × this.
    pub slowness: f32,
    /// The bob shows at all (first person, the switch on, nothing else driving the view).
    pub on: bool,
    /// The breathing camera shows (first person, its own switch, nothing else driving
    /// the view).
    pub breathing: bool,
}

/// The bob's state across frames.
#[derive(Debug, Clone, Default)]
pub struct HeadBob {
    amount: f32,
    gain: f32,
    /// The bob after its spring (view frame: x right, z up), the head's lag (x right, y
    /// ahead) and its dip, and what pushed them last frame.
    bob: [Spring; 2],
    lag: [Spring; 2],
    land: Spring,
    last_velocity: Option<Vec2>,
    last_vertical: f32,
    /// The deepest take-off or landing dip shown since the start (for the exit summary).
    pub peak_land: f32,
    /// The breath: where in its cycle (0–1), how much shows, and the most rise shown.
    breath_phase: f32,
    breath: f32,
    pub peak_breath: f32,
    /// The third-person pivot's two lags (head height above the feet).
    pivot: Option<(f32, f32)>,
    /// The deepest dip shown since the start (for the exit summary).
    pub peak_dip: f32,
    /// Third person, for the exit summary: the head's and the pivot's height above the
    /// feet last frame, and the sums of their squared changes per frame over `frames`.
    last: Option<(f32, f32)>,
    head_moves: f64,
    pivot_moves: f64,
    pub frames: u32,
}

impl HeadBob {
    /// The first-person rig's offset this frame, game space.
    pub fn first_person(&mut self, i: BobInput) -> Vec3 {
        let dt = i.dt.max(0.0);
        let follow = |t: f32| if dt <= 0.0 { 0.0 } else { 1.0 - (-dt / t).exp() };
        self.gain += (f32::from(u8::from(i.on)) - self.gain) * follow(FOLLOW_SECONDS);
        let target = if i.airborne { 0.0 } else { (i.speed / FULL_SPEED).min(1.0) };
        self.amount += (target - self.amount) * follow(FOLLOW_SECONDS);
        let share = self.gain * self.amount * if i.sights { SIGHTS } else { 1.0 } * if i.sneaking { SNEAK } else { 1.0 };
        let a = std::f32::consts::TAU * (i.phase - crate::view_sway::FOOTFALL_PHASE);
        // Down at each footfall (twice a cycle), across once a cycle, through the spring.
        let slow = i.slowness.max(0.1);
        let w = omega_for(BOB_SETTLE_SECONDS * slow);
        self.bob[0].step(SWING * a.sin() * share, w, dt);
        self.bob[1].step(-DIP * 0.5 * (1.0 + (2.0 * a).cos()) * share, w, dt);
        let (swing, dip) = (self.bob[0].value, self.bob[1].value);
        self.peak_dip = self.peak_dip.max(-dip);
        // The head's own weight, pushed by the body's acceleration.
        let accel = match (self.last_velocity, dt > 0.0) {
            (Some(v), true) => (i.velocity - v) / dt,
            _ => Vec2::ZERO,
        };
        self.last_velocity = Some(i.velocity);
        let up_accel = if dt > 0.0 { (i.vertical - self.last_vertical) / dt } else { 0.0 };
        self.last_vertical = i.vertical;
        let lag_w = LAG_OMEGA / slow;
        for (s, a) in self.lag.iter_mut().zip([accel.x, accel.y]) {
            s.step(pushed(a, lag_w, LAG_PEAK_PER_SPEED), lag_w, dt);
        }
        let land_w = LAND_OMEGA / slow;
        self.land.step(pushed(up_accel, land_w, LAND_PEAK_PER_SPEED), land_w, dt);
        let calm = self.gain * if i.sights { 0.5 } else { 1.0 };
        let lag = Vec2::new(self.lag[0].value, self.lag[1].value);
        let lag = if lag.length() > 1e-6 { lag * (soft(lag.length(), MAX_LAG) / lag.length()) * calm } else { Vec2::ZERO };
        let land = soft(self.land.value, MAX_LAND) * calm;
        self.peak_land = self.peak_land.max(-land);
        // Breathing: a slow, small rise and fall, a little forward with it.
        let breath_target = if !i.breathing || i.airborne {
            0.0
        } else {
            (if i.speed < STILL_SPEED { 1.0 } else { BREATH_MOVING }) * if i.sights { 0.5 } else { 1.0 }
        };
        self.breath += (breath_target - self.breath) * follow(BREATH_FADE_SECONDS);
        self.breath_phase = (self.breath_phase + dt / BREATH_SECONDS).fract();
        let b = std::f32::consts::TAU * self.breath_phase;
        let rise = BREATH_RISE * b.sin() * self.breath;
        let lean = BREATH_AHEAD * (b - std::f32::consts::FRAC_PI_2).sin() * self.breath;
        self.peak_breath = self.peak_breath.max(rise);
        // The view's right and ahead: heading clockwise from +y.
        let right = Vec3::new(i.heading.cos(), -i.heading.sin(), 0.0);
        let ahead = Vec3::new(i.heading.sin(), i.heading.cos(), 0.0);
        right * (swing + lag.x) + ahead * (lag.y + lean) + Vec3::Z * (dip + land + rise)
    }

    /// The third-person pivot's height above the feet this frame, from the head's.
    pub fn pivot_height(&mut self, head: f32, dt: f32) -> f32 {
        let k = if dt <= 0.0 { 0.0 } else { 1.0 - (-dt / PIVOT_SECONDS).exp() };
        let (a, b) = match self.pivot {
            Some((a, b)) if (head - b).abs() <= PIVOT_SNAP => {
                let a = a + (head - a) * k;
                (a, b + (a - b) * k)
            }
            _ => (head, head),
        };
        self.pivot = Some((a, b));
        if let Some((h, p)) = self.last {
            self.head_moves += f64::from(head - h).powi(2);
            self.pivot_moves += f64::from(b - p).powi(2);
            self.frames += 1;
        }
        self.last = Some((head, b));
        b
    }

    /// RMS change per frame of the head's and the pivot's height above the feet in third
    /// person, units.
    pub fn pivot_rms(&self) -> (f32, f32) {
        let n = f64::from(self.frames.max(1));
        ((self.head_moves / n).sqrt() as f32, (self.pivot_moves / n).sqrt() as f32)
    }

    /// Forget the pivot (furniture, a view without the head): it starts at the head next.
    pub fn pivot_reset(&mut self) {
        self.pivot = None;
        self.last = None;
    }
}

/// The target of a spring of rate `omega` pushed by `accel`, scaled so a sudden change `Δ`
/// of the velocity behind it peaks at `peak_per_speed × Δ` (the impulse response of a
/// critically damped spring peaks at `1/(ω e)`).
fn pushed(accel: f32, omega: f32, peak_per_speed: f32) -> f32 {
    -peak_per_speed * std::f32::consts::E * accel / omega
}

/// `x` towards `max` and never past it, smoothly.
fn soft(x: f32, max: f32) -> f32 {
    max * (x / max).tanh()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn input(speed: f32, phase: f32) -> BobInput {
        BobInput { dt: DT, speed, phase, heading: 0.0, sights: false, sneaking: false, airborne: false, velocity: Vec2::ZERO, vertical: 0.0, slowness: 1.0, on: true, breathing: false }
    }

    #[test]
    fn the_breathing_camera_rises_and_falls_slowly_and_only_a_little() {
        let mut b = HeadBob::default();
        let zs: Vec<f32> = (0..(12.0 / DT) as usize).map(|_| b.first_person(BobInput { breathing: true, ..input(0.0, 0.0) }).z).collect();
        // After it has faded in: up and down by about its rise, once per breath.
        let late = &zs[(4.0 / DT) as usize..];
        let (lo, hi) = late.iter().fold((f32::MAX, f32::MIN), |(l, h), z| (l.min(*z), h.max(*z)));
        assert!(hi > 0.8 * BREATH_RISE && hi <= BREATH_RISE + 1e-4 && lo < -0.8 * BREATH_RISE, "{lo} {hi}");
        let tops = late.windows(3).filter(|w| w[1] > w[0] && w[1] >= w[2]).count();
        assert_eq!(tops, ((late.len() as f32 * DT) / BREATH_SECONDS).round() as usize, "one rise a breath");
        // Smooth: no frame moves it more than a sliver.
        assert!(zs.windows(2).all(|w| (w[1] - w[0]).abs() < 0.02));
        // Off: nothing.
        let mut b = HeadBob::default();
        for _ in 0..600 {
            assert!(b.first_person(input(0.0, 0.0)).length() < 1e-6);
        }
    }

    #[test]
    fn the_head_lags_behind_a_start_and_comes_back_at_a_steady_pace() {
        let mut b = HeadBob::default();
        let mut most = 0.0f32;
        for f in 0..120 {
            // The body speeds up over half a second (`nvf_play::inertia`), heading north.
            let v = 344.0 * (f as f32 / 30.0).min(1.0);
            let o = b.first_person(BobInput { velocity: Vec2::new(0.0, v), ..input(0.0, 0.0) });
            most = most.min(o.y);
        }
        assert!((-MAX_LAG..-0.2).contains(&most), "behind while speeding up: {most}");
        let mut o = Vec3::ONE;
        for _ in 0..120 {
            o = b.first_person(BobInput { velocity: Vec2::new(0.0, 344.0), ..input(0.0, 0.0) });
        }
        assert!(o.y.abs() < 1e-2, "back over the feet at a steady pace: {o}");
    }

    /// A jump's vertical speeds frame by frame and the landing: the deepest dip after it.
    fn landing_dip(land_speed: f32, slowness: f32) -> (f32, usize) {
        let mut b = HeadBob::default();
        let (g, up) = (686.7f32, 290.9f32);
        let mut vz = up;
        while vz > -land_speed {
            b.first_person(BobInput { vertical: vz, airborne: true, slowness, ..input(0.0, 0.0) });
            vz -= g * DT;
        }
        let mut deepest = (0.0f32, 0);
        for f in 0..60 {
            let z = b.first_person(BobInput { slowness, ..input(0.0, 0.0) }).z;
            if z < deepest.0 {
                deepest = (z, f);
            }
        }
        deepest
    }

    #[test]
    fn a_landing_dips_the_view_more_the_harder_it_is() {
        let (hop, at) = landing_dip(291.0, 1.0);
        let (fall, _) = landing_dip(800.0, 1.0);
        assert!(hop < -0.3 && hop > -MAX_LAND, "{hop}");
        assert!(fall < hop - 0.3 && fall >= -MAX_LAND, "{fall} against {hop}");
        let (_, slow_at) = landing_dip(291.0, 2.0);
        assert!(slow_at > at, "twice the inertia weight, a slower dip: frame {slow_at} against {at}");
    }

    #[test]
    fn standing_still_does_not_bob() {
        let mut b = HeadBob::default();
        let mut o = Vec3::ZERO;
        for f in 0..120 {
            o = b.first_person(input(0.0, f as f32 / 72.0));
        }
        assert!(o.length() < 1e-4, "{o}");
    }

    #[test]
    fn running_dips_twice_a_stride() {
        let mut b = HeadBob::default();
        let mut zs = Vec::new();
        // 1.2 s cycle, two seconds of running.
        for f in 0..240 {
            zs.push(b.first_person(input(300.0, (f as f32 * DT / 1.2).fract())).z);
        }
        // Frames 101-239: the footfalls (phases 0.33 and 0.83 of the 72-frame cycle) at
        // frames 132, 168 and 204.
        let last = &zs[101..];
        let low = last.iter().cloned().fold(0.0, f32::min);
        assert!((-DIP..-0.9 * DIP).contains(&low), "deepest {low}");
        let turns = last.windows(3).filter(|w| w[1] < w[0] && w[1] <= w[2]).count();
        assert_eq!(turns, 3, "{last:?}");
    }

    #[test]
    fn switched_off_fades_out() {
        let mut b = HeadBob::default();
        for f in 0..120 {
            b.first_person(input(300.0, f as f32 / 72.0));
        }
        let mut o = Vec3::ONE;
        for f in 0..120 {
            o = b.first_person(BobInput { on: false, ..input(300.0, f as f32 / 72.0) });
        }
        assert!(o.length() < 0.01, "{o}");
    }

    #[test]
    fn the_third_person_pivot_leaves_out_the_stride_but_follows_a_crouch() {
        let mut b = HeadBob::default();
        // The head rises and falls 3 units twice a 1.2 s stride about 120.
        let mut out = Vec::new();
        for f in 0..240 {
            let t = f as f32 * DT;
            out.push(b.pivot_height(120.0 + 3.0 * (std::f32::consts::TAU * 2.0 * t / 1.2).sin(), DT));
        }
        let tail = &out[120..];
        let (lo, hi) = tail.iter().fold((f32::MAX, f32::MIN), |(l, h), x| (l.min(*x), h.max(*x)));
        assert!(hi - lo < 0.15 * 6.0, "pivot moved {} of the head's 6", hi - lo);
        // Crouching: the head drops 30 units; 90 % of the way in a second.
        let mut p = 0.0;
        for _ in 0..60 {
            p = b.pivot_height(90.0, DT);
        }
        assert!(p < 93.0, "after a second {p}");
    }

    #[test]
    fn a_teleport_moves_the_pivot_at_once() {
        let mut b = HeadBob::default();
        b.pivot_height(120.0, DT);
        assert_eq!(b.pivot_height(30.0, DT), 30.0);
    }
}
