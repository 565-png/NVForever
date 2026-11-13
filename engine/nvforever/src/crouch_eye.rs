//! The first-person view sinks into a crouch and rises from it on an ease-out curve
//! [design: the owner's movement-inertia request of 2026-11-13, "Crouching - same as jump
//! but slower as it has to be realistic speed"; on with the "Movement inertia" switch].
//!
//! The first-person sneak clips lower `Bip01 NonAccum`, and with it the camera node, by
//! [`SNEAK_EYE_DROP`] [observed: animation.md §9.5]. The clips cross-fade from standing to
//! sneaking and back as the animator fades layers: the clip asked for in at its fade-in
//! time, the other out over `DEFAULT_FADE`, a straight ramp (`nvf_assets::anim_blend`).
//! So the eye drops at a constant speed and stops dead.
//!
//! This keeps the clips as they are and moves the arms rig (camera node, arms and weapon
//! together) by the difference between where the clips' cross-fade has the eye and where
//! a cubic ease-out (`nvf_play::ease::EaseOut`) would: about [`DOWN_SECONDS`] down and
//! [`UP_SECONDS`] up, fast at first and settling softly. A crouch pressed again on the way
//! turns back from where the view is. Settled in either stance the difference is nothing,
//! so a mistake in the model of the cross-fade only ever shows during the move.

use nvf_assets::anim_blend::DEFAULT_FADE;
use nvf_play::ease::EaseOut;

/// How far the sneak clips lower the first-person camera node, units at scale 1 (118 up
/// standing, 78 crouched) [observed: animation.md §9.5].
pub const SNEAK_EYE_DROP: f32 = 40.0;
/// Seconds to sink all the way into a crouch, and to rise from one.
pub const DOWN_SECONDS: f32 = 0.35;
pub const UP_SECONDS: f32 = 0.4;

/// The crouch's state across frames.
#[derive(Debug, Clone, Default)]
pub struct CrouchEye {
    /// The clips' cross-fade as the animator runs it: the standing and the sneaking
    /// clips' fades (0–1); `None` before the first frame.
    clips: Option<[f32; 2]>,
    /// How far down the eased view is (0 standing, 1 crouched).
    eased: EaseOut,
}

impl CrouchEye {
    /// One frame of `dt` seconds: `sneaking` asked for, the clip asked for fading in over
    /// `fade_in` seconds, the easing `on` or not, the rig drawn at `scale`. The height to
    /// add to the arms rig, units (up).
    pub fn update(&mut self, sneaking: bool, fade_in: f32, on: bool, scale: f32, dt: f32) -> f32 {
        let want = usize::from(sneaking);
        if self.clips.is_none() {
            // The first frame: settled in the stance the game starts in.
            self.eased = EaseOut::at(want as f32);
        }
        let clips = self.clips.get_or_insert_with(|| {
            let mut c = [0.0; 2];
            c[want] = 1.0;
            c
        });
        let dt = dt.max(0.0);
        for (i, e) in clips.iter_mut().enumerate() {
            *e = if i == want {
                if fade_in <= 0.0 {
                    1.0
                } else {
                    (*e + dt / fade_in).min(1.0)
                }
            } else {
                (*e - dt / DEFAULT_FADE).max(0.0)
            };
        }
        let total = clips[0] + clips[1];
        let down = if total > 0.0 { clips[1] / total } else { want as f32 };
        if on {
            let seconds = if sneaking { DOWN_SECONDS } else { UP_SECONDS };
            self.eased.retarget(want as f32, seconds, 1.0);
            self.eased.step(dt);
        } else {
            // Off: the view goes with the clips, and the curve stands ready where they are.
            self.eased = EaseOut::at(down);
            self.eased.retarget(want as f32, 0.0, 1.0);
        }
        SNEAK_EYE_DROP * scale * (down - self.eased.value)
    }

    /// How far down the view is now (0 standing, 1 crouched), for the log.
    pub fn share(&self) -> f32 {
        self.eased.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    /// The eye's height below standing over `frames`: the clips' cross-fade plus the
    /// offset.
    fn run(c: &mut CrouchEye, sneaking: bool, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|_| {
                let lift = c.update(sneaking, DEFAULT_FADE, true, 1.0, DT);
                let clips = c.clips.unwrap();
                let down = clips[1] / (clips[0] + clips[1]);
                SNEAK_EYE_DROP * down - lift
            })
            .collect()
    }

    #[test]
    fn settled_it_adds_nothing() {
        let mut c = CrouchEye::default();
        assert_eq!(c.update(false, DEFAULT_FADE, true, 1.0, DT), 0.0);
        let mut c = CrouchEye::default();
        assert_eq!(c.update(true, DEFAULT_FADE, true, 1.0, DT), 0.0, "a crouched start stays crouched");
        run(&mut c, false, 60);
        assert!(c.update(false, DEFAULT_FADE, true, 1.0, DT).abs() < 1e-5);
    }

    #[test]
    fn the_view_sinks_fast_at_first_and_settles_softly_in_its_time() {
        let mut c = CrouchEye::default();
        run(&mut c, false, 10);
        let drop = run(&mut c, true, 60);
        let steps: Vec<f32> = std::iter::once(drop[0]).chain(drop.windows(2).map(|w| w[1] - w[0])).collect();
        assert!(steps.iter().all(|s| *s >= -1e-4), "only down: {drop:?}");
        assert!(steps.windows(2).all(|w| w[1] <= w[0] + 1e-3), "never faster than before: {steps:?}");
        // Faster than the clips' straight 0.2 s at first, and a soft arrival.
        assert!(drop[2] > SNEAK_EYE_DROP * 3.0 / 12.0, "{drop:?}");
        let done = (DOWN_SECONDS / DT).round() as usize;
        assert!((drop[done] - SNEAK_EYE_DROP).abs() < 0.05, "down in time: {}", drop[done]);
        assert!(drop[done - 3] > SNEAK_EYE_DROP - 0.5, "softly: {}", drop[done - 3]);
        assert!(*drop.last().unwrap() == SNEAK_EYE_DROP);
    }

    #[test]
    fn standing_up_takes_longer_and_a_change_of_mind_turns_back_smoothly() {
        let mut c = CrouchEye::default();
        run(&mut c, true, 60);
        let up = run(&mut c, false, 60);
        let done = (UP_SECONDS / DT).round() as usize;
        assert!(up[done - 3] > 0.0 && up[done].abs() < 0.05, "{up:?}");
        // Halfway down, back up: no jump in height.
        let mut c = CrouchEye::default();
        run(&mut c, false, 10);
        let down = run(&mut c, true, 6);
        let back = run(&mut c, false, 60);
        let at = *down.last().unwrap();
        assert!((back[0] - at).abs() < 6.0 && back[0] <= at, "{at} then {}", back[0]);
        assert!(back.last().unwrap().abs() < 1e-4);
    }

    #[test]
    fn off_it_follows_the_clips_and_scales_with_the_body() {
        let mut c = CrouchEye::default();
        c.update(false, DEFAULT_FADE, false, 1.0, DT);
        for _ in 0..30 {
            assert_eq!(c.update(true, DEFAULT_FADE, false, 1.0, DT), 0.0);
        }
        let mut a = CrouchEye::default();
        let mut b = CrouchEye::default();
        a.update(false, DEFAULT_FADE, true, 1.0, DT);
        b.update(false, DEFAULT_FADE, true, 0.5, DT);
        let x = a.update(true, DEFAULT_FADE, true, 1.0, DT);
        let y = b.update(true, DEFAULT_FADE, true, 0.5, DT);
        assert!((y - 0.5 * x).abs() < 1e-5 && x != 0.0);
    }
}
