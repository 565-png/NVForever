//! Whether the landscape is smoothed (#2149, `fnv_world::land_surface`): one switch for
//! the whole run, so the drawn ground (nvforever terrain.rs), its collision
//! (`crate::phys`) and every height query stand on the same surface.
//!
//! [design, #2149] On by default (maintainer decision 2026-11-13: "terrain smoothing should
//! be the default have the collision follow the smoothed surface"); `--no-smooth-terrain`
//! or the settings page's Smooth terrain row turns it off. A change on the page takes
//! effect at the next start (loading a save starts the game again): cells are drawn and
//! collided as they load, so a change while the game runs would leave the loaded cells'
//! ground and collision on the old surface beside new cells on the other [design].
//!
//! How it is smoothed is fixed for the run too ([`params`]): by default interpolating,
//! 2 × 2, within 8 units (`fnv_world::land_surface::SmoothParams::DEFAULT`). Three test aids
//! choose another recipe for comparing them in the game [design, #2149: the maintainer
//! picks a default from such runs; they are not settings rows or INI entries]:
//! - `NVF_PLAY_TEST_SMOOTH_MODE=interpolate|fit` (the LAND vertices kept, or fitted);
//! - `NVF_PLAY_TEST_SMOOTH_SUBDIV=2|4` (new squares per grid square and side);
//! - `NVF_PLAY_TEST_SMOOTH_BOUND=<units>` (0 to 16: how far from the stored triangles).
//!
//! A value out of range gives one warning at the start and that field's default. With
//! smoothing off they do nothing: the run draws and collides the stored triangles.

use fnv_world::land_surface::{SmoothMode, SmoothParams, SurfaceSpec, MAX_BOUND, SUBDIVS};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// What the settings page shows and the INI keeps (the next start's state).
static SETTING: AtomicBool = AtomicBool::new(true);
/// This run's state, fixed once the INI is read ([`fix`]).
static RUN: AtomicBool = AtomicBool::new(true);
/// This run's recipe, read once from the test aids ([`params`]).
static PARAMS: OnceLock<SmoothParams> = OnceLock::new();

/// The INI key and settings row.
pub const KEY: &str = "smooth-terrain";

/// The test aids (module notes).
pub const MODE_AID: &str = "NVF_PLAY_TEST_SMOOTH_MODE";
pub const SUBDIV_AID: &str = "NVF_PLAY_TEST_SMOOTH_SUBDIV";
pub const BOUND_AID: &str = "NVF_PLAY_TEST_SMOOTH_BOUND";

/// The settings row's note, NVForever's own words [design].
pub const NOTE: &str = "On: the ground between the landscape's points is drawn and walked on as a smooth surface, at most one height step from the stored ground. Off: the stored triangles. Takes effect at the next start.";

/// The state the settings page shows (what the next start uses).
pub fn setting() -> bool {
    SETTING.load(Ordering::Relaxed)
}

pub fn set_setting(on: bool) {
    SETTING.store(on, Ordering::Relaxed);
}

/// Whether this run's landscape is smoothed.
pub fn on() -> bool {
    RUN.load(Ordering::Relaxed)
}

/// Fix the run's state from the setting: before the first cell loads (once at start from
/// the command line, again at `PostStartup` after the INI is applied).
pub fn fix() {
    RUN.store(setting(), Ordering::Relaxed);
}

/// This run's smoothing recipe from the test aids (module notes), read once; a bad value
/// warns and takes that field's default.
pub fn params() -> SmoothParams {
    *PARAMS.get_or_init(|| {
        let var = |k: &str| std::env::var(k).ok();
        let (p, problems) = parse_params(var(MODE_AID).as_deref(), var(SUBDIV_AID).as_deref(), var(BOUND_AID).as_deref());
        for e in problems {
            bevy::log::warn!("{e}");
        }
        p
    })
}

/// The surface this run draws, collides with and measures on: what every job that builds
/// one carries (it never reads the settings itself).
pub fn spec() -> SurfaceSpec {
    if on() {
        SurfaceSpec::Smoothed(params())
    } else {
        SurfaceSpec::Stored
    }
}

/// The recipe the test aids' values give (`None`: not set), and a line for each value
/// that was not taken (that field keeps its default).
pub fn parse_params(mode: Option<&str>, subdiv: Option<&str>, bound: Option<&str>) -> (SmoothParams, Vec<String>) {
    let mut p = SmoothParams::DEFAULT;
    let mut problems = Vec::new();
    if let Some(v) = mode.map(str::trim) {
        match v.to_ascii_lowercase().as_str() {
            "interpolate" => p.mode = SmoothMode::Interpolate,
            "fit" => p.mode = SmoothMode::Fit,
            _ => problems.push(format!("{MODE_AID}={v:?}: expected interpolate or fit; using {}", p.mode.name())),
        }
    }
    if let Some(v) = subdiv.map(str::trim) {
        match v.parse::<usize>() {
            Ok(n) if SUBDIVS.contains(&n) => p.subdiv = n,
            _ => problems.push(format!("{SUBDIV_AID}={v:?}: expected one of {SUBDIVS:?}; using {}", p.subdiv)),
        }
    }
    if let Some(v) = bound.map(str::trim) {
        match v.parse::<f32>() {
            Ok(b) if b.is_finite() && (0.0..=MAX_BOUND).contains(&b) => p.max_deviation = b,
            _ => problems.push(format!("{BOUND_AID}={v:?}: expected units from 0 to {MAX_BOUND}; using {}", p.max_deviation)),
        }
    }
    debug_assert!(p.is_valid());
    (p, problems)
}

/// The start's one log line for the surface (the smoke scenarios `terrain-smoothing*`
/// read it).
pub fn describe() -> String {
    describe_spec(spec(), params())
}

fn describe_spec(spec: SurfaceSpec, params: SmoothParams) -> String {
    let mut line = format!("terrain: {}; drawing, collision and queries use this surface", spec.describe());
    if !spec.is_smoothed() {
        line.push_str(" (smoothing off: --no-smooth-terrain or the Smooth terrain setting");
        if params != SmoothParams::DEFAULT {
            line.push_str("; the NVF_PLAY_TEST_SMOOTH_* recipe is not used");
        }
        line.push(')');
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_aids_choose_the_recipe() {
        assert_eq!(parse_params(None, None, None), (SmoothParams::DEFAULT, Vec::new()));
        let (p, e) = parse_params(Some("fit"), Some("4"), Some("12"));
        assert_eq!((p, e.len()), (SmoothParams { mode: SmoothMode::Fit, subdiv: 4, max_deviation: 12.0 }, 0));
        let (p, e) = parse_params(Some(" Interpolate "), Some("2"), Some("0"));
        assert_eq!((p, e.len()), (SmoothParams { mode: SmoothMode::Interpolate, subdiv: 2, max_deviation: 0.0 }, 0));
        assert_eq!(parse_params(None, None, Some("2.5")).0.max_deviation, 2.5);
    }

    /// Each bad field warns once and keeps its default; the good ones are still taken.
    #[test]
    fn bad_values_fall_back_field_by_field() {
        for (mode, subdiv, bound) in [("bicubic", "4", "12"), ("fit", "3", "12"), ("fit", "4", "-1"), ("fit", "4", "NaN"), ("fit", "4", "inf"), ("fit", "4", "16.5"), ("fit", "x", "12")] {
            let (p, e) = parse_params(Some(mode), Some(subdiv), Some(bound));
            assert_eq!(e.len(), 1, "{mode} {subdiv} {bound}: {e:?}");
            assert!(p.is_valid());
            let want = SmoothParams {
                mode: if mode == "fit" { SmoothMode::Fit } else { SmoothMode::Interpolate },
                subdiv: if subdiv == "4" { 4 } else { 2 },
                max_deviation: if bound == "12" { 12.0 } else { 8.0 },
            };
            assert_eq!(p, want, "{mode} {subdiv} {bound}");
        }
        let (_, e) = parse_params(Some("x"), Some("x"), Some("x"));
        assert_eq!(e.len(), 3);
        assert!(e[0].starts_with(MODE_AID) && e[1].starts_with(SUBDIV_AID) && e[2].starts_with(BOUND_AID));
    }

    #[test]
    fn the_start_line_names_the_surface() {
        assert_eq!(describe_spec(SurfaceSpec::DEFAULT, SmoothParams::DEFAULT), "terrain: mode=interpolate subdiv=2 bound=8 land-shift=0; drawing, collision and queries use this surface");
        let fit = SmoothParams { mode: SmoothMode::Fit, subdiv: 4, max_deviation: 16.0 };
        assert_eq!(describe_spec(SurfaceSpec::Smoothed(fit), fit), "terrain: mode=fit subdiv=4 bound=16 land-shift=4; drawing, collision and queries use this surface");
        assert_eq!(
            describe_spec(SurfaceSpec::Stored, SmoothParams::DEFAULT),
            "terrain: mode=stored subdiv=1 bound=0 land-shift=0; drawing, collision and queries use this surface (smoothing off: --no-smooth-terrain or the Smooth terrain setting)"
        );
        assert!(describe_spec(SurfaceSpec::Stored, fit).ends_with("; the NVF_PLAY_TEST_SMOOTH_* recipe is not used)"));
    }
}
