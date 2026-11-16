//! What frame pacing's cap is divided by while DLSS Frame Generation runs (#2317,
//! LootJunkieAzza/NVForever#2731): the frames DLSS-G presents per real frame, not the frames
//! asked for. `frame_gen.rs` `state` feeds it each present's `numFramesActuallyPresented`
//! [documented: `ProgrammingGuideDLSS_G.md` §13] with whether the mode was on for it and the
//! frames presented per real frame asked for (the options' generated frames plus one); the
//! divisor goes to `frame_pace.rs` `wait_for_cap` through `dlss::frame_generation_divisor`, which
//! hands Reflex the cap divided by it.
//!
//! Why: DLSS-G presents one frame per real frame whenever it does not generate, and the cap
//! divided by the asked frames then holds the real frames, which are all that is shown, at a
//! fraction of the limit. That happens in menus, loading and the pause (the mode off), and when
//! Streamline stops interpolating by itself while its status stays `eOk` (the window not in front:
//! "DLSS-G disabled: window not focused", 480 real frames and 480 presented in 10 s at a
//! limit of 48) [observed: a player's logs in #2731].
//!
//! **The states** [design]:
//!
//! - [`Phase::Waiting`] (divisor 1, the full limit): at the start, after DLSS-G is loaded again,
//!   and after the mode was off for [`MENU_FRAMES`] real frames in a row ("menus": menus,
//!   loading, the pause). The first present with the mode on starts generation:
//!   [`Phase::Generating`] at once, divided by the frames asked for, before any frame has
//!   shown generation (the bootstrap: DLSS-G generates whatever the cap, so dividing at once only
//!   saves the first frames a burst at the full limit).
//! - [`Phase::Generating`] (divisor: the frames asked for until [`WINDOW`] real frames have been
//!   seen, then the mean presented over the frames of the last [`WINDOW`] that showed generated
//!   ones, rounded, kept between 2 and the frames asked for, and moved only when the mean is
//!   [`HYSTERESIS`] or more away, so a mixed run does not flap it: DLSS-G presenting 3 where 4
//!   were asked divides by 3). A frame presented once now and then does not end it: only
//!   [`PAUSE_FRAMES`] real frames in a row presented once ("presented 1 per real frame for N
//!   frames": the window left the front, or generation never started), or fewer than half of
//!   the last [`WINDOW`] showing generated frames, go to [`Phase::Paused`]; the mode off for
//!   [`MENU_FRAMES`] in a row goes to [`Phase::Waiting`]. A new number of frames asked for (the
//!   Generated frames row) divides by it at once and starts the window again.
//! - [`Phase::Paused`] (divisor 1, the full limit, the mode still on): [`RESUME_FRAMES`] real
//!   frames in a row presented twice or more ("generation resumed") go back to
//!   [`Phase::Generating`]; the mode off for [`MENU_FRAMES`] goes to [`Phase::Waiting`] without
//!   a change of the divisor.
//!
//! A present that showed nothing (0: the window minimised, an image that came back outdated)
//! counts for none of these. A window change or the live switch holds the mode off for one to
//! three frames (`frame_gen.rs` `window_changes`), under [`MENU_FRAMES`], so it changes nothing.

use std::collections::VecDeque;

/// Real frames in a row with DLSS-G's mode off before the cap is the full limit ("menus").
pub(super) const MENU_FRAMES: u32 = 8;
/// Real frames in a row presented once, with the mode on, before the cap is the full limit.
pub(super) const PAUSE_FRAMES: u32 = 30;
/// Real frames in a row presented twice or more, while paused, before the cap divides again.
pub(super) const RESUME_FRAMES: u32 = 4;
/// Real frames the mean presented is taken over while generating.
pub(super) const WINDOW: usize = 32;
/// How far the mean must be from the divisor before the divisor follows it.
const HYSTERESIS: f64 = 0.75;

/// Where the divisor stands ([`CapDivisor`]; the module comment).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Waiting,
    Generating,
    Paused,
}

/// One present, as the state after it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Present {
    /// The mode was on for it (the options last set).
    pub(super) mode_on: bool,
    /// `numFramesActuallyPresented`: real and generated frames presented for it.
    pub(super) presented: u32,
    /// Frames presented per real frame asked for: the options' generated frames plus one.
    pub(super) asked: u32,
}

/// A change of the divisor and why, for the log ([`Change::line`]).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Change {
    pub(super) from: u32,
    pub(super) to: u32,
    pub(super) reason: String,
}

impl Change {
    /// The log line (after `DLSS-G: `).
    pub(super) fn line(&self) -> String {
        let what = if self.to <= 1 { "the full limit".to_string() } else { format!("the limit divided by {}", self.to) };
        format!("frame cap: real frames at {what}, was {} ({})", if self.from <= 1 { "the full limit".to_string() } else { format!("divided by {}", self.from) }, self.reason)
    }
}

/// The divisor's state machine (the module comment).
#[derive(Clone, Debug)]
pub(super) struct CapDivisor {
    phase: Phase,
    divisor: u32,
    /// The frames asked for that the divisor was set from in [`Phase::Generating`].
    asked: u32,
    off_run: u32,
    once_run: u32,
    more_run: u32,
    window: VecDeque<u32>,
}

impl Default for CapDivisor {
    fn default() -> Self {
        Self { phase: Phase::Waiting, divisor: 1, asked: 1, off_run: 0, once_run: 0, more_run: 0, window: VecDeque::with_capacity(WINDOW) }
    }
}

impl CapDivisor {
    /// What the cap is divided by now (1: the full limit).
    pub(super) fn divisor(&self) -> u32 {
        self.divisor
    }

    #[cfg(test)]
    pub(super) fn phase(&self) -> Phase {
        self.phase
    }

    /// The state after one present; the change of the divisor, if any.
    pub(super) fn after(&mut self, p: Present) -> Option<Change> {
        if p.presented == 0 {
            return None;
        }
        // With the mode on at least one frame is generated (`frame_gen.rs` clamps the options'
        // frames to 1 and up), so at least two are asked for; kept so for the clamp below.
        let asked = p.asked.max(2);
        if !p.mode_on {
            self.once_run = 0;
            self.more_run = 0;
            self.off_run = self.off_run.saturating_add(1);
            if self.off_run >= MENU_FRAMES && self.phase != Phase::Waiting {
                let was = self.phase;
                self.phase = Phase::Waiting;
                if was == Phase::Generating {
                    return self.set(1, format!("menus: DLSS-G's mode off for {MENU_FRAMES} frames"));
                }
            }
            return None;
        }
        self.off_run = 0;
        match self.phase {
            Phase::Waiting => self.generate(asked, "generation starting: DLSS-G's mode on".to_string()),
            Phase::Paused => {
                self.more_run = if p.presented >= 2 { self.more_run + 1 } else { 0 };
                if self.more_run >= RESUME_FRAMES {
                    return self.generate(asked, "generation resumed".to_string());
                }
                None
            }
            Phase::Generating => {
                if asked != self.asked {
                    return self.generate(asked, format!("{asked} frames asked for per real frame"));
                }
                self.once_run = if p.presented <= 1 { self.once_run + 1 } else { 0 };
                if self.window.len() == WINDOW {
                    self.window.pop_front();
                }
                self.window.push_back(p.presented);
                if self.once_run >= PAUSE_FRAMES {
                    return self.pause(format!("presented 1 per real frame for {PAUSE_FRAMES} frames"));
                }
                if self.window.len() < WINDOW {
                    return None;
                }
                let generated: Vec<u32> = self.window.iter().copied().filter(|&n| n >= 2).collect();
                if generated.len() < WINDOW / 2 {
                    return self.pause(format!("presented 1 per real frame for {} of the last {WINDOW} frames", WINDOW - generated.len()));
                }
                let mean = f64::from(generated.iter().sum::<u32>()) / generated.len() as f64;
                let to = (mean.round() as u32).clamp(2, asked);
                if to != self.divisor && (mean - f64::from(self.divisor)).abs() >= HYSTERESIS {
                    return self.set(to, format!("presented {mean:.2} per generating real frame over the last {WINDOW} frames"));
                }
                None
            }
        }
    }

    /// Into [`Phase::Paused`]: the full limit.
    fn pause(&mut self, reason: String) -> Option<Change> {
        self.phase = Phase::Paused;
        self.more_run = 0;
        self.set(1, reason)
    }

    /// Into [`Phase::Generating`], divided by `asked`, the window started again.
    fn generate(&mut self, asked: u32, reason: String) -> Option<Change> {
        self.phase = Phase::Generating;
        self.asked = asked;
        self.once_run = 0;
        self.more_run = 0;
        self.window.clear();
        self.set(asked, reason)
    }

    fn set(&mut self, to: u32, reason: String) -> Option<Change> {
        let from = std::mem::replace(&mut self.divisor, to);
        (from != to).then_some(Change { from, to, reason })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(presented: u32, asked: u32) -> Present {
        Present { mode_on: true, presented, asked }
    }

    fn off() -> Present {
        Present { mode_on: false, presented: 1, asked: 4 }
    }

    /// The divisor after each of `presents`, and the changes said.
    fn run(c: &mut CapDivisor, presents: impl IntoIterator<Item = Present>) -> Vec<Change> {
        presents.into_iter().filter_map(|p| c.after(p)).collect()
    }

    #[test]
    fn all_four_divides_by_four_from_the_first_frame() {
        let mut c = CapDivisor::default();
        assert_eq!(c.divisor(), 1, "the full limit before any present");
        let changes = run(&mut c, std::iter::repeat_n(on(4, 4), 200));
        assert_eq!(c.divisor(), 4);
        assert_eq!(changes.len(), 1, "said once: {changes:?}");
        assert_eq!(changes[0].line(), "frame cap: real frames at the limit divided by 4, was the full limit (generation starting: DLSS-G's mode on)");
    }

    #[test]
    fn the_bootstrap_divides_while_generation_starts_and_falls_back_when_it_never_does() {
        let mut c = CapDivisor::default();
        // The first frames with the mode on present once (the options act at the next present).
        let changes = run(&mut c, [on(1, 3), on(1, 3), on(3, 3), on(3, 3)]);
        assert_eq!((c.divisor(), c.phase()), (3, Phase::Generating), "divided while generation starts");
        assert_eq!(changes.len(), 1);
        // A start where it never generates (the window not in front): the full limit after
        // PAUSE_FRAMES presented once, said with the count.
        let mut c = CapDivisor::default();
        // The bootstrap's own frame and then PAUSE_FRAMES - 1 presented once: not yet.
        let changes = run(&mut c, std::iter::repeat_n(on(1, 4), PAUSE_FRAMES as usize));
        assert_eq!(c.divisor(), 4, "not yet: {changes:?}");
        let changes = run(&mut c, [on(1, 4)]);
        assert_eq!((c.divisor(), c.phase()), (1, Phase::Paused));
        assert_eq!(changes[0].line(), "frame cap: real frames at the full limit, was divided by 4 (presented 1 per real frame for 30 frames)");
    }

    #[test]
    fn all_ones_while_on_is_the_full_limit_until_generation_resumes() {
        let mut c = CapDivisor::default();
        run(&mut c, std::iter::repeat_n(on(4, 4), 100));
        // Streamline stops interpolating (the window not in front), the status still eOk.
        let changes = run(&mut c, std::iter::repeat_n(on(1, 4), 480));
        assert_eq!(c.divisor(), 1);
        assert_eq!(changes.len(), 1, "one line for the pause: {changes:?}");
        // One generated frame now and then does not divide again.
        run(&mut c, [on(4, 4), on(1, 4), on(4, 4), on(4, 4), on(1, 4)]);
        assert_eq!(c.divisor(), 1);
        let changes = run(&mut c, std::iter::repeat_n(on(4, 4), RESUME_FRAMES as usize));
        assert_eq!(c.divisor(), 4);
        assert_eq!(changes[0].reason, "generation resumed");
    }

    #[test]
    fn a_mixed_run_follows_the_mean_without_flapping() {
        let mut c = CapDivisor::default();
        // Now and then a frame presented once: still divided by four.
        let changes = run(&mut c, (0..400).map(|i| on(if i % 16 == 7 { 1 } else { 4 }, 4)));
        assert_eq!((c.divisor(), changes.len()), (4, 1), "{changes:?}");
        // One in four presented once: the others still show three generated frames each.
        let changes = run(&mut c, (0..400).map(|i| on(if i % 4 == 3 { 1 } else { 4 }, 4)));
        assert_eq!((c.divisor(), changes.len()), (4, 0), "{changes:?}");
        // DLSS-G presenting three where four were asked: divided by three, moved once.
        let changes = run(&mut c, std::iter::repeat_n(on(3, 4), 400));
        assert_eq!(c.divisor(), 3, "{changes:?}");
        assert_eq!(changes.len(), 1, "moved once, no flapping: {changes:?}");
        assert!(changes[0].reason.starts_with("presented 3."), "{}", changes[0].reason);
        // Means around 3.5 (between 3 and 4) do not move it either way.
        let changes = run(&mut c, (0..400).map(|i| on(if i % 2 == 0 { 3 } else { 4 }, 4)));
        assert_eq!((c.divisor(), changes.len()), (3, 0), "{changes:?}");
        // Generation in one frame of three: fewer than half the window, the full limit.
        let mut c = CapDivisor::default();
        let changes = run(&mut c, (0..400).map(|i| on(if i % 3 == 0 { 2 } else { 1 }, 2)));
        assert_eq!((c.divisor(), c.phase()), (1, Phase::Paused), "{changes:?}");
        assert!(changes.last().is_some_and(|c| c.reason.ends_with("of the last 32 frames")), "{changes:?}");
        assert_eq!(changes.len(), 2, "divided at the start, the full limit once: {changes:?}");
    }

    #[test]
    fn menus_go_to_the_full_limit_and_play_divides_again() {
        let mut c = CapDivisor::default();
        run(&mut c, std::iter::repeat_n(on(3, 3), 100));
        // A window change holds the mode off for a frame or three: nothing changes.
        let changes = run(&mut c, [off(), off(), off(), on(3, 3)]);
        assert_eq!((c.divisor(), changes.len()), (3, 0));
        // The pause menu: the full limit after MENU_FRAMES, said once.
        let changes = run(&mut c, std::iter::repeat_n(off(), 300));
        assert_eq!(c.divisor(), 1);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].line(), "frame cap: real frames at the full limit, was divided by 3 (menus: DLSS-G's mode off for 8 frames)");
        // Back in play: divided at once (the bootstrap).
        let changes = run(&mut c, [on(1, 3)]);
        assert_eq!(c.divisor(), 3);
        assert_eq!(changes[0].reason, "generation starting: DLSS-G's mode on");
        // Paused (not in front), then a menu: no line for the menu, the next play divides.
        run(&mut c, std::iter::repeat_n(on(1, 3), PAUSE_FRAMES as usize));
        assert_eq!(c.divisor(), 1);
        let changes = run(&mut c, std::iter::repeat_n(off(), 20));
        assert!(changes.is_empty(), "{changes:?}");
        assert_eq!(c.phase(), Phase::Waiting);
        run(&mut c, [on(1, 3)]);
        assert_eq!(c.divisor(), 3);
    }

    #[test]
    fn a_new_choice_divides_by_it_at_once_and_nothing_presented_counts_for_nothing() {
        let mut c = CapDivisor::default();
        run(&mut c, std::iter::repeat_n(on(2, 2), 100));
        let changes = run(&mut c, [on(2, 4)]);
        assert_eq!(c.divisor(), 4);
        assert_eq!(changes[0].reason, "4 frames asked for per real frame");
        // A minimised window presents nothing: neither a pause nor a menu.
        let changes = run(&mut c, std::iter::repeat_n(Present { mode_on: true, presented: 0, asked: 4 }, 500));
        assert!(changes.is_empty());
        let changes = run(&mut c, std::iter::repeat_n(Present { mode_on: false, presented: 0, asked: 4 }, 500));
        assert!(changes.is_empty());
        assert_eq!(c.divisor(), 4);
    }
}
