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
//! How it is smoothed is fixed for the run too ([`params`]): by default detail,
//! 4 × 4, within 8 units, thresholds 2:6 (`fnv_world::land_surface::SmoothParams::DEFAULT`).
//! [design, #2149: the contributor's pick from in-game comparisons (2026-11-15), for the maintainer to confirm.] Test aids
//! choose another recipe for comparing them in the game; they are not settings rows
//! or INI entries:
//! - `NVF_PLAY_TEST_SMOOTH_MODE=interpolate|fit|detail` (the LAND vertices kept, fitted,
//!   or fitted where they look like quantisation steps only);
//! - `NVF_PLAY_TEST_SMOOTH_DETAIL=<t0>:<t1>` (detail's thresholds, units, 0 ≤ t0 < t1 ≤ 16;
//!   2:6 by default; used with `detail` only);
//! - `NVF_PLAY_TEST_SMOOTH_SUBDIV=2|4` (new squares per grid square and side);
//! - `NVF_PLAY_TEST_SMOOTH_BOUND=<units>` (0 to 16: how far from the stored triangles).
//!
//! A value out of range gives one warning at the start and that field's default. With
//! smoothing off they do nothing: the run draws and collides the stored triangles.

use fnv_world::land_surface::{detail_thresholds_valid, SmoothMode, SmoothParams, SurfaceSpec, DEFAULT_DETAIL, MAX_BOUND, SUBDIVS};
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
pub const DETAIL_AID: &str = "NVF_PLAY_TEST_SMOOTH_DETAIL";

/// The settings row's note, NVForever's own words [design].
pub const NOTE: &str = "Smooths height steps while keeping bumps and ridges, with 4 x 4 squares per landscape square and an 8-unit bound. Drawing and collision share the surface. Applies at the next start.";

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
        let (p, problems) = parse_params(var(MODE_AID).as_deref(), var(SUBDIV_AID).as_deref(), var(BOUND_AID).as_deref(), var(DETAIL_AID).as_deref());
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
pub fn parse_params(mode: Option<&str>, subdiv: Option<&str>, bound: Option<&str>, detail: Option<&str>) -> (SmoothParams, Vec<String>) {
    let mut p = SmoothParams::DEFAULT;
    let mut problems = Vec::new();
    if let Some(v) = mode.map(str::trim) {
        match v.to_ascii_lowercase().as_str() {
            "interpolate" => p.mode = SmoothMode::Interpolate,
            "fit" => p.mode = SmoothMode::Fit,
            "detail" => p.mode = SmoothMode::DETAIL,
            _ => problems.push(format!("{MODE_AID}={v:?}: expected interpolate, fit or detail; using {}", p.mode.name())),
        }
    }
    if let Some(v) = detail.map(str::trim) {
        let parsed = v.split_once(':').and_then(|(a, b)| Some((a.trim().parse::<f32>().ok()?, b.trim().parse::<f32>().ok()?)));
        match (parsed, p.mode) {
            (Some((t0, t1)), SmoothMode::Detail { .. }) if detail_thresholds_valid(t0, t1) => p.mode = SmoothMode::Detail { t0, t1 },
            (Some((t0, t1)), _) if detail_thresholds_valid(t0, t1) => problems.push(format!("{DETAIL_AID}={v:?} is used with {MODE_AID}=detail only; not used")),
            _ => problems.push(format!("{DETAIL_AID}={v:?}: expected <t0>:<t1>, units, 0 <= t0 < t1 <= {MAX_BOUND}; using {}:{}", DEFAULT_DETAIL.0, DEFAULT_DETAIL.1)),
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
        assert_eq!(parse_params(None, None, None, None), (SmoothParams::DEFAULT, Vec::new()));
        let (p, e) = parse_params(Some("fit"), Some("4"), Some("12"), None);
        assert_eq!((p, e.len()), (SmoothParams { mode: SmoothMode::Fit, subdiv: 4, max_deviation: 12.0 }, 0));
        let (p, e) = parse_params(Some(" Interpolate "), Some("2"), Some("0"), None);
        assert_eq!((p, e.len()), (SmoothParams { mode: SmoothMode::Interpolate, subdiv: 2, max_deviation: 0.0 }, 0));
        assert_eq!(parse_params(None, None, Some("2.5"), None).0.max_deviation, 2.5);
    }

    /// Detail and its thresholds: the default 2:6, another pair, a bad pair (warned,
    /// the default), a pair without detail (warned, not used).
    #[test]
    fn detail_takes_its_thresholds() {
        let (p, e) = parse_params(Some("detail"), Some("4"), Some("8"), None);
        assert_eq!((p, e.len()), (SmoothParams { mode: SmoothMode::DETAIL, subdiv: 4, max_deviation: 8.0 }, 0));
        assert_eq!(p.mode, SmoothMode::Detail { t0: 2.0, t1: 6.0 });
        let (p, e) = parse_params(Some("detail"), None, None, Some(" 1.5 : 9 "));
        assert_eq!((p.mode, e.len()), (SmoothMode::Detail { t0: 1.5, t1: 9.0 }, 0));
        // Detail is also the default when the mode aid is absent.
        assert_eq!(parse_params(None, None, None, Some(" 1.5 : 9 ")), (p, Vec::new()));
        for bad in ["6:2", "3:3", "-1:4", "2:17", "2", "a:b", "NaN:4", "2:inf"] {
            let (p, e) = parse_params(Some("detail"), None, None, Some(bad));
            assert_eq!((p.mode, e.len()), (SmoothMode::DETAIL, 1), "{bad}");
            assert!(e[0].starts_with(DETAIL_AID) && e[0].contains("expected"), "{bad}: {e:?}");
        }
        let (p, e) = parse_params(Some("fit"), None, None, Some("1:5"));
        assert_eq!((p.mode, e.len()), (SmoothMode::Fit, 1));
        assert!(e[0].contains("detail only"));
        let detail = SmoothParams { mode: SmoothMode::Detail { t0: 1.5, t1: 9.0 }, subdiv: 4, max_deviation: 8.0 };
        assert_eq!(describe_spec(SurfaceSpec::Smoothed(detail), detail), "terrain: mode=detail(1.5:9) subdiv=4 bound=8 land-shift=4; drawing, collision and queries use this surface");
    }

    /// Each bad field warns once and keeps its default; the good ones are still taken.
    #[test]
    fn bad_values_fall_back_field_by_field() {
        for (mode, subdiv, bound) in [("bicubic", "4", "12"), ("fit", "3", "12"), ("fit", "4", "-1"), ("fit", "4", "NaN"), ("fit", "4", "inf"), ("fit", "4", "16.5"), ("fit", "x", "12")] {
            let (p, e) = parse_params(Some(mode), Some(subdiv), Some(bound), None);
            assert_eq!(e.len(), 1, "{mode} {subdiv} {bound}: {e:?}");
            assert!(p.is_valid());
            let want = SmoothParams {
                mode: if mode == "fit" { SmoothMode::Fit } else { SmoothParams::DEFAULT.mode },
                subdiv: if subdiv == "4" { 4 } else { SmoothParams::DEFAULT.subdiv },
                max_deviation: if bound == "12" { 12.0 } else { 8.0 },
            };
            assert_eq!(p, want, "{mode} {subdiv} {bound}");
        }
        let (_, e) = parse_params(Some("x"), Some("x"), Some("x"), None);
        assert_eq!(e.len(), 3);
        assert!(e[0].starts_with(MODE_AID) && e[1].starts_with(SUBDIV_AID) && e[2].starts_with(BOUND_AID));
    }

    #[test]
    fn the_start_line_names_the_surface() {
        assert_eq!(describe_spec(SurfaceSpec::DEFAULT, SmoothParams::DEFAULT), "terrain: mode=detail(2:6) subdiv=4 bound=8 land-shift=4; drawing, collision and queries use this surface");
        let fit = SmoothParams { mode: SmoothMode::Fit, subdiv: 4, max_deviation: 16.0 };
        assert_eq!(describe_spec(SurfaceSpec::Smoothed(fit), fit), "terrain: mode=fit subdiv=4 bound=16 land-shift=4; drawing, collision and queries use this surface");
        assert_eq!(
            describe_spec(SurfaceSpec::Stored, SmoothParams::DEFAULT),
            "terrain: mode=stored subdiv=1 bound=0 land-shift=0; drawing, collision and queries use this surface (smoothing off: --no-smooth-terrain or the Smooth terrain setting)"
        );
        assert!(describe_spec(SurfaceSpec::Stored, fit).ends_with("; the NVF_PLAY_TEST_SMOOTH_* recipe is not used)"));
    }
}
