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
//! **The rules** [design]. Only presents that showed something count (0: the window minimised,
//! an image that came back outdated, changes nothing).
//!
//! - **Waiting** (divisor 1, the full limit): at the start, after DLSS-G is loaded or unloaded,
//!   and after the mode was off for [`MENU_FRAMES`] real frames in a row ("menus": menus,
//!   loading, the pause). A window change or the live switch holds the mode off for one to three
//!   frames (`frame_gen.rs` `window_changes`), under [`MENU_FRAMES`], so it changes nothing.
//! - **Starting**: the first present with the mode on after Waiting divides by the frames asked
//!   for at once, before any frame has shown generation (the bootstrap: the cap does not decide
//!   whether DLSS-G generates, so dividing at once only spares the first frames a burst at the
//!   full limit). A new number of frames asked for (the Generated frames row, up or down) does
//!   the same. Both start the history again.
//! - **Following the rate**: the history is the last [`WINDOW`] presents with the mode on, every
//!   count in it (those presented once too). Its mean, rounded (half up), between 1 and the
//!   frames asked for, is what the divisor moves to, when the mean is [`HYSTERESIS`] or more from
//!   the divisor, the history is full, the present itself generated (so a run of presents shown
//!   once never steps it down frame by frame), and [`MIN_DWELL`] presents have passed since the
//!   last change. A divisor of 1 is the full limit (generation paused); the same rule divides
//!   again once the history's mean has climbed to 1.75 ("generation resumed"), so a resume needs
//!   generation sustained over a good part of the history, and the history is not started again
//!   by a pause or a resume: an intermittent pattern that does not change settles on one divisor.
//!   Steady 4, 1, 4, 1 at 4x averages 2.5 and divides by 3.
//! - **A quick pause**: while divided, [`PAUSE_RUN`] presents in a row shown once (the window
//!   left the front, or generation never started) go to the full limit at once ("presented 1 per
//!   real frame for N frames"), without waiting for the mean. Each quick pause doubles the run
//!   the next one needs (up to [`PAUSE_RUN_MAX`]), so bursts of generation and runs shown once
//!   repeating leave it to the mean; [`QUIET_FRAMES`] presents with no run of [`PAUSE_RUN`] / 2
//!   shown once bring it back to [`PAUSE_RUN`].
//!
//! Each change of the divisor is one `frame cap:` line with its reason ([`Change::line`]).

use std::collections::VecDeque;

/// Real frames in a row with DLSS-G's mode off before the cap is the full limit ("menus").
pub(super) const MENU_FRAMES: u32 = 8;
/// Presents in a row shown once, while divided, for a quick pause (doubled by each one).
pub(super) const PAUSE_RUN: u32 = 16;
/// The most the quick pause's run grows to.
pub(super) const PAUSE_RUN_MAX: u32 = 256;
/// Presents with no run of `PAUSE_RUN / 2` shown once before the quick pause's run is
/// [`PAUSE_RUN`] again.
pub(super) const QUIET_FRAMES: u32 = 600;
/// Presents the mean is taken over.
pub(super) const WINDOW: usize = 32;
/// Presents after a change before the mean may move the divisor again.
pub(super) const MIN_DWELL: u32 = 16;
/// How far the mean must be from the divisor before the divisor follows it.
const HYSTERESIS: f64 = 0.75;

/// Where the divisor stands ([`CapDivisor`]; the module comment), for the tests.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    /// The full limit until the mode is on.
    Waiting,
    /// Divided (2 and up).
    Generating,
    /// The mode on, the full limit (divisor 1).
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
    waiting: bool,
    divisor: u32,
    /// The frames asked for at the last start.
    asked: u32,
    off_run: u32,
    once_run: u32,
    /// Presents since the divisor last changed.
    dwell: u32,
    /// The run of presents shown once a quick pause needs now.
    pause_run: u32,
    /// Presents since a run of `PAUSE_RUN / 2` shown once.
    quiet: u32,
    window: VecDeque<u32>,
}

impl Default for CapDivisor {
    fn default() -> Self {
        Self { waiting: true, divisor: 1, asked: 1, off_run: 0, once_run: 0, dwell: 0, pause_run: PAUSE_RUN, quiet: 0, window: VecDeque::with_capacity(WINDOW) }
    }
}

impl CapDivisor {
    /// What the cap is divided by now (1: the full limit); the changes carry it otherwise.
    #[cfg(test)]
    pub(super) fn divisor(&self) -> u32 {
        self.divisor
    }

    #[cfg(test)]
    pub(super) fn phase(&self) -> Phase {
        match (self.waiting, self.divisor) {
            (true, _) => Phase::Waiting,
            (false, 1) => Phase::Paused,
            _ => Phase::Generating,
        }
    }

    /// The state after one present; the change of the divisor, if any.
    pub(super) fn after(&mut self, p: Present) -> Option<Change> {
        if p.presented == 0 {
            return None;
        }
        if !p.mode_on {
            self.once_run = 0;
            self.off_run = self.off_run.saturating_add(1);
            if self.off_run >= MENU_FRAMES && !self.waiting {
                self.waiting = true;
                return self.set(1, format!("menus: DLSS-G's mode off for {MENU_FRAMES} frames"));
            }
            return None;
        }
        self.off_run = 0;
        // With the mode on at least one frame is generated (`frame_gen.rs` clamps the options'
        // frames to 1 and up), so at least two are asked for.
        let asked = p.asked.max(2);
        if self.waiting {
            return self.start(asked, "generation starting: DLSS-G's mode on".to_string());
        }
        if asked != self.asked {
            return self.start(asked, format!("{asked} frames asked for per real frame"));
        }
        self.dwell = self.dwell.saturating_add(1);
        self.once_run = if p.presented <= 1 { self.once_run + 1 } else { 0 };
        self.quiet = if self.once_run >= PAUSE_RUN / 2 { 0 } else { self.quiet.saturating_add(1) };
        if self.quiet >= QUIET_FRAMES {
            self.pause_run = PAUSE_RUN;
        }
        if self.window.len() == WINDOW {
            self.window.pop_front();
        }
        self.window.push_back(p.presented);
        if self.divisor >= 2 && self.once_run >= self.pause_run {
            let run = self.once_run;
            self.pause_run = (self.pause_run * 2).min(PAUSE_RUN_MAX);
            return self.set(1, format!("presented 1 per real frame for {run} frames"));
        }
        if self.window.len() < WINDOW || self.once_run > 0 || self.dwell < MIN_DWELL {
            return None;
        }
        let mean = f64::from(self.window.iter().sum::<u32>()) / WINDOW as f64;
        let to = (mean.round() as u32).clamp(1, asked);
        if to == self.divisor || (mean - f64::from(self.divisor)).abs() < HYSTERESIS {
            return None;
        }
        let head = if self.divisor == 1 { "generation resumed: " } else { "" };
        self.set(to, format!("{head}presented {mean:.2} per real frame over the last {WINDOW} frames"))
    }

    /// Divided by `asked` at once, the history started again.
    fn start(&mut self, asked: u32, reason: String) -> Option<Change> {
        self.waiting = false;
        self.asked = asked;
        self.once_run = 0;
        self.window.clear();
        self.set(asked, reason)
    }

    fn set(&mut self, to: u32, reason: String) -> Option<Change> {
        self.dwell = 0;
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

    /// The changes said over `presents`.
    fn run(c: &mut CapDivisor, presents: impl IntoIterator<Item = Present>) -> Vec<Change> {
        presents.into_iter().filter_map(|p| c.after(p)).collect()
    }

    /// `pattern` repeated over `n` presents at `asked`.
    fn repeat(pattern: &[u32], n: usize, asked: u32) -> impl Iterator<Item = Present> + '_ {
        pattern.iter().cycle().take(n).map(move |&p| on(p, asked))
    }

    #[test]
    fn all_four_divides_by_four_from_the_first_frame() {
        let mut c = CapDivisor::default();
        assert_eq!(c.divisor(), 1, "the full limit before any present");
        let changes = run(&mut c, repeat(&[4], 200, 4));
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
        // PAUSE_RUN presented once after the bootstrap's own present, said with the count.
        let mut c = CapDivisor::default();
        let changes = run(&mut c, repeat(&[1], PAUSE_RUN as usize, 4));
        assert_eq!(c.divisor(), 4, "not yet: {changes:?}");
        let changes = run(&mut c, [on(1, 4)]);
        assert_eq!((c.divisor(), c.phase()), (1, Phase::Paused));
        assert_eq!(changes[0].line(), "frame cap: real frames at the full limit, was divided by 4 (presented 1 per real frame for 16 frames)");
    }

    #[test]
    fn all_ones_while_on_is_the_full_limit_until_generation_resumes() {
        let mut c = CapDivisor::default();
        run(&mut c, repeat(&[4], 100, 4));
        // Streamline stops interpolating (the window not in front), the status still eOk: one
        // quick pause, no steps down on the way.
        let changes = run(&mut c, repeat(&[1], 480, 4));
        assert_eq!(c.divisor(), 1);
        assert_eq!(changes.len(), 1, "one line for the pause: {changes:?}");
        // A generated frame now and then does not divide again.
        let changes = run(&mut c, repeat(&[4, 1, 1, 1, 1, 1, 1, 1], 200, 4));
        assert_eq!((c.divisor(), changes.len()), (1, 0), "{changes:?}");
        // Generation back for good: divided again, up to four, each step a line.
        let changes = run(&mut c, repeat(&[4], 200, 4));
        assert_eq!(c.divisor(), 4);
        assert!(changes[0].reason.starts_with("generation resumed: presented "), "{changes:?}");
        assert!(changes.windows(2).all(|w| w[1].to > w[0].to) && changes.len() <= 3, "{changes:?}");
    }

    #[test]
    fn a_mixed_run_follows_the_mean_without_flapping() {
        let mut c = CapDivisor::default();
        // Now and then a frame presented once: still divided by four.
        let changes = run(&mut c, (0..400).map(|i| on(if i % 16 == 7 { 1 } else { 4 }, 4)));
        assert_eq!((c.divisor(), changes.len()), (4, 1), "{changes:?}");
        // One in four presented once: 3.25 a real frame, divided by three, moved once.
        let changes = run(&mut c, repeat(&[4, 4, 4, 1], 400, 4));
        assert_eq!((c.divisor(), changes.len()), (3, 1), "{changes:?}");
        assert!(changes[0].reason.starts_with("presented 3.25 per real frame over the last 32 frames"), "{}", changes[0].reason);
        // DLSS-G presenting three where four were asked: divided by three.
        let mut c = CapDivisor::default();
        let changes = run(&mut c, repeat(&[3], 400, 4));
        assert_eq!((c.divisor(), changes.len()), (3, 2), "{changes:?}");
        // Means around 3.5 (between 3 and 4) do not move it either way.
        let changes = run(&mut c, repeat(&[3, 4], 400, 4));
        assert_eq!((c.divisor(), changes.len()), (3, 0), "{changes:?}");
        // Generation in one frame of five at 2x: 1.2 a real frame, the full limit.
        let mut c = CapDivisor::default();
        let changes = run(&mut c, (0..400).map(|i| on(if i % 5 == 0 { 2 } else { 1 }, 2)));
        assert_eq!((c.divisor(), c.phase()), (1, Phase::Paused), "{changes:?}");
        assert_eq!(changes.len(), 2, "divided at the start, the full limit once: {changes:?}");
    }

    /// Review #2731 r1: 4, 1, 4, 1 at 4x presents 2.5 a real frame, not 4.
    #[test]
    fn steady_four_one_divides_by_the_rate_presented() {
        let mut c = CapDivisor::default();
        let changes = run(&mut c, repeat(&[4, 1], 400, 4));
        assert_eq!(c.divisor(), 3, "2.5 rounds half up: {changes:?}");
        assert_eq!(changes.len(), 2, "the start, then once: {changes:?}");
        assert!(changes[1].reason.starts_with("presented 2.50 per real frame"), "{changes:?}");
        let changes = run(&mut c, repeat(&[4, 1], 2000, 4));
        assert!(changes.is_empty(), "settled: {changes:?}");
    }

    /// Review #2731 r1: an intermittent pattern that does not change settles in one state
    /// instead of pausing and resuming over and over.
    #[test]
    fn bursts_and_runs_shown_once_settle() {
        // Four generated, five shown once.
        let mut c = CapDivisor::default();
        let changes = run(&mut c, repeat(&[4, 4, 4, 4, 1, 1, 1, 1, 1], 900, 4));
        assert!(changes.len() <= 3, "{changes:?}");
        assert!((2..=3).contains(&c.divisor()), "{changes:?}");
        let changes = run(&mut c, repeat(&[4, 4, 4, 4, 1, 1, 1, 1, 1], 3000, 4));
        assert!(changes.is_empty(), "settled: {changes:?}");
        // Sixteen generated, sixteen shown once: one quick pause, then the mean, which a run of
        // sixteen no longer pauses (the run doubled) and which keeps it divided.
        let mut c = CapDivisor::default();
        let pattern: Vec<u32> = [4; 16].into_iter().chain([1; 16]).collect();
        let changes = run(&mut c, repeat(&pattern, 640, 4));
        assert!(changes.len() <= 3, "{changes:?}");
        let changes = run(&mut c, repeat(&pattern, 3200, 4));
        assert!(changes.is_empty(), "settled, longer than QUIET_FRAMES too: {changes:?}");
        assert!(c.divisor() >= 2);
    }

    /// Review #2731 r1: after a pause, an alternating pattern resumes once and stays.
    #[test]
    fn a_pause_then_alternating_resumes_once() {
        let mut c = CapDivisor::default();
        run(&mut c, repeat(&[4], 100, 4));
        run(&mut c, repeat(&[1], 200, 4));
        assert_eq!(c.divisor(), 1);
        let changes = run(&mut c, repeat(&[4, 1], 400, 4));
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert!(changes[0].reason.starts_with("generation resumed: "), "{changes:?}");
        assert!((2..=3).contains(&c.divisor()));
        let changes = run(&mut c, repeat(&[4, 1], 2000, 4));
        assert!(changes.is_empty(), "settled: {changes:?}");
        // A later loss of focus, after a quiet spell, pauses quickly again.
        let mut c = CapDivisor::default();
        run(&mut c, repeat(&[4], 100, 4));
        run(&mut c, repeat(&[1], 100, 4));
        run(&mut c, repeat(&[4], QUIET_FRAMES as usize + 100, 4));
        assert_eq!(c.divisor(), 4);
        let changes = run(&mut c, repeat(&[1], PAUSE_RUN as usize, 4));
        assert_eq!((c.divisor(), changes.len()), (1, 1), "{changes:?}");
    }

    #[test]
    fn menus_go_to_the_full_limit_and_play_divides_again() {
        let mut c = CapDivisor::default();
        run(&mut c, repeat(&[3], 100, 3));
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
        run(&mut c, repeat(&[1], PAUSE_RUN as usize, 3));
        assert_eq!(c.divisor(), 1);
        let changes = run(&mut c, std::iter::repeat_n(off(), 20));
        assert!(changes.is_empty(), "{changes:?}");
        assert_eq!(c.phase(), Phase::Waiting);
        run(&mut c, [on(1, 3)]);
        assert_eq!(c.divisor(), 3);
    }

    #[test]
    fn a_new_choice_divides_by_it_at_once_up_or_down() {
        let mut c = CapDivisor::default();
        run(&mut c, repeat(&[2], 100, 2));
        let changes = run(&mut c, [on(2, 4)]);
        assert_eq!(c.divisor(), 4);
        assert_eq!(changes[0].reason, "4 frames asked for per real frame");
        run(&mut c, repeat(&[4], 100, 4));
        // Down: 4x to 2x, divided by two at once, the history of fours forgotten.
        let changes = run(&mut c, [on(4, 2)]);
        assert_eq!(c.divisor(), 2);
        assert_eq!(changes[0].reason, "2 frames asked for per real frame");
        let changes = run(&mut c, repeat(&[2], 200, 2));
        assert!(changes.is_empty(), "{changes:?}");
        assert_eq!(c.divisor(), 2);
    }

    #[test]
    fn nothing_presented_counts_for_nothing() {
        let mut c = CapDivisor::default();
        run(&mut c, repeat(&[4], 100, 4));
        // A minimised window presents nothing: neither a pause nor a menu.
        let changes = run(&mut c, std::iter::repeat_n(Present { mode_on: true, presented: 0, asked: 4 }, 500));
        assert!(changes.is_empty());
        let changes = run(&mut c, std::iter::repeat_n(Present { mode_on: false, presented: 0, asked: 4 }, 500));
        assert!(changes.is_empty());
        assert_eq!(c.divisor(), 4);
    }
}
