//! Test aid `NVF_PLAY_TEST_TERRAIN_PARALLAX_AB=<seconds>[:hold][,…]` (#3322, terrain parallax;
//! terrain.rs "Parallax", docs/spec-notes/pbr-materials.md "Terrain layers: parallax"): a
//! readback of the world view that tells whether the terrain parallax moves the ground's
//! texture under the camera, for a smoke scenario over generated ground sets (nvf_assets
//! `NVF_PLAY_TEST_PBR_GROUND_PACK`). `--play` only; it makes the run unattended.
//!
//! At each time (on the world aids' clock, nvf-play test_aid.rs "Which clock"), with the view
//! posed by the other aids (`NVF_PLAY_TEST_LOOK`), three screenshots of the primary window are
//! taken, A, B and C:
//! - by default (`flip`) with the Terrain parallax switch off, on, off: the aid sets
//!   `Graphics::terrain_parallax` as the settings row does (settings.rs), and puts the run's
//!   own value back after C;
//! - with `:hold` the switch is not touched: three pictures of the run as it is.
//!
//! Each shot waits until [`SETTLE_SECS`] have passed since its switch state was set and the
//! pipeline gate counts no pipeline still compiling (pipeline_gate.rs; a variant asked for by
//! the flip is drawn only once it is ready), at most [`CAP_SECS`] (then the verdict is
//! `unsettled`). A shot is requested only after the previous one was received, and the next
//! state is set only then, so no shot shows a state set in its own frame. The pictures are
//! saved as `terrain_parallax_ab_<seconds>s_<a|b|c>.png` in `NVF_PLAY_TEST_SHOT_DIR`.
//!
//! The measure ([`measure`]), on each picture's brightness (the mean of the three colour
//! channels, 0..255) in a small box at the centre of the view ([`centre_box`]), the
//! crosshair's square left out ([`CROSS_PX`]): A-B and A-C, the mean absolute difference
//! (A-C, two pictures with the same switch state, is the floor of what changes anyway:
//! grass, clouds, light); the whole-pixel translation that best matches B to A ([`best_shift`]:
//! B(x, y) ≈ A(x - dx, y - dy), searched within [`search_radius`]) and the difference left at
//! it; each picture's contrast. The verdict ([`judge`]):
//! - `moved`: B is A translated by at least [`MOVED_PX`], the translation explains most of the
//!   difference, and the difference stands well above the floor;
//! - `still`: B differs from A no more than the floor allows, or the best translation is at
//!   most [`STILL_PX`] and the difference a small share of the picture's contrast
//!   ([`STILL_SHARE`]; head-on, the march's shift is nothing at the centre and grows radially
//!   with the view's tilt away from it, so it has no common translation);
//! - `flat` (a picture without texture to measure), `unsettled`, or `unclear` otherwise.
//!
//! [design expectation, to confirm with the first real run] Ground seen at a grazing angle
//! sinks under the march (a height map below its top lies under the surface), so a pixel
//! shows the texture of a point further along its ray: the picture's texture moves towards the
//! camera, down the screen (dy > 0) with no turn about the view (dx about 0). Head-on the view
//! vector has no tangent part at the centre: no shift there.
//!
//! Thresholds [engineering policy]: set from reasoning about the expected values (module
//! constants), with wide margins, not from readbacks; the main session confirms them against
//! the first real ones. Nothing here is a fact about the original game.

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured};
use nvf_play::pause::AidTime;
use std::collections::VecDeque;
use std::path::PathBuf;

/// The aid's variable.
pub const ENV: &str = "NVF_PLAY_TEST_TERRAIN_PARALLAX_AB";

/// The least time between setting a switch state and its shot (seconds): the flipped
/// materials' edit reaches the GPU, the eye adaptation and any temporal filter settle.
pub const SETTLE_SECS: f32 = 2.0;

/// The most time a shot waits for the pipeline gate (seconds).
pub const CAP_SECS: f32 = 20.0;

/// The measured box: this share of the view's width and height, centred.
pub const BOX_SHARE: (f32, f32) = (0.06, 0.08);

/// Half the side of the square at the centre left out of the measure (the crosshair), pixels.
pub const CROSS_PX: i32 = 12;

/// The least translation (pixels, the larger component) a `moved` verdict needs.
pub const MOVED_PX: i32 = 3;

/// The largest translation (pixels, the larger component) a `still` verdict allows.
pub const STILL_PX: i32 = 1;

/// A `moved` verdict needs A-B at least this many levels and this many times A-C.
pub const MOVED_LEVELS: f64 = 1.5;
pub const MOVED_RATIO: f64 = 2.0;

/// A `moved` verdict needs the difference left at the best translation no more than this
/// share of A-B: the translation explains the change.
pub const MOVED_FIT: f64 = 0.7;

/// B counts as unchanged from A when A-B is at most this many levels, or this many times A-C.
pub const STILL_LEVELS: f64 = 1.0;
pub const STILL_RATIO: f64 = 2.0;

/// A `still` verdict by translation (at most [`STILL_PX`]) needs A-B no more than this share of
/// A's contrast: a picture changed altogether differs by about 1.1 times its spread (two
/// unrelated samples of one spread), one whose texture moves by a fraction of its grain by
/// much less.
pub const STILL_SHARE: f64 = 0.6;

/// A picture whose box has a brightness spread (standard deviation) under this many levels
/// has nothing to measure a translation by.
pub const FLAT_LEVELS: f64 = 2.0;

/// One timed sequence: when, and whether the switch is left alone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spec {
    pub at: f32,
    pub hold: bool,
}

/// `<seconds>[:hold][,…]`, in time order.
pub fn parse(value: &str) -> Result<Vec<Spec>, String> {
    let mut specs = Vec::new();
    for item in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (time, mode) = item.split_once(':').map_or((item, None), |(t, m)| (t, Some(m)));
        let at: f32 = time.trim().parse().map_err(|_| format!("{item:?}: a time in seconds first"))?;
        let hold = match mode.map(str::trim) {
            None | Some("flip") => false,
            Some("hold") => true,
            Some(m) => return Err(format!("{item:?}: {m:?} is neither flip nor hold")),
        };
        specs.push(Spec { at, hold });
    }
    if specs.is_empty() {
        return Err("no time given".into());
    }
    specs.sort_by(|a, b| a.at.total_cmp(&b.at));
    Ok(specs)
}

/// One picture's brightness, row by row.
#[derive(Debug, Clone, Default)]
pub struct Luma {
    pub w: usize,
    pub h: usize,
    pub v: Vec<f32>,
}

impl Luma {
    /// From four bytes a pixel, row by row (RGBA or BGRA alike: the three colour channels'
    /// mean). `None` for any other size.
    pub fn from_pixels(bytes: &[u8], w: usize, h: usize) -> Option<Luma> {
        if w == 0 || h == 0 || bytes.len() != w * h * 4 {
            return None;
        }
        let v = bytes.as_chunks::<4>().0.iter().map(|p| (f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2])) / 3.0).collect();
        Some(Luma { w, h, v })
    }

    fn at(&self, x: i32, y: i32) -> f32 {
        self.v[y as usize * self.w + x as usize]
    }
}

/// A box of the picture: left, top, width, height (pixels).
pub type Rect = (i32, i32, i32, i32);

/// The measured box for a `w` x `h` view ([`BOX_SHARE`], centred, at least 8 pixels a side).
pub fn centre_box(w: usize, h: usize) -> Rect {
    let bw = ((w as f32 * BOX_SHARE.0) as i32).max(8);
    let bh = ((h as f32 * BOX_SHARE.1) as i32).max(8);
    ((w as i32 - bw) / 2, (h as i32 - bh) / 2, bw, bh)
}

/// The translation searched, at most, for a `w` x `h` view: a 40th of the width sideways, a
/// little more than an eighth of the height up and down (the grazing view's shift runs along
/// the screen's vertical), both held inside the picture around [`centre_box`].
pub fn search_radius(w: usize, h: usize) -> (i32, i32) {
    let (x0, y0, bw, bh) = centre_box(w, h);
    let rx = (w as i32 / 40).min(x0).min(w as i32 - x0 - bw);
    let ry = (h as i32 * 2 / 15).min(y0).min(h as i32 - y0 - bh);
    (rx.max(0), ry.max(0))
}

/// Whether a pixel lies in the crosshair's square, left out of the measure.
fn crosshair(l: &Luma, x: i32, y: i32) -> bool {
    (x - l.w as i32 / 2).abs() < CROSS_PX && (y - l.h as i32 / 2).abs() < CROSS_PX
}

/// The mean absolute difference between `b` over `rect` and `a` translated by `(dx, dy)`
/// (`b(x, y)` against `a(x - dx, y - dy)`), every `step`th pixel, the crosshair left out on
/// both sides. `None` when no pixel was compared or the translation leaves `a`.
pub fn difference(a: &Luma, b: &Luma, rect: Rect, (dx, dy): (i32, i32), step: usize) -> Option<f64> {
    let (x0, y0, bw, bh) = rect;
    let (mut sum, mut n) = (0.0f64, 0u32);
    for y in (y0..y0 + bh).step_by(step) {
        for x in (x0..x0 + bw).step_by(step) {
            let (ax, ay) = (x - dx, y - dy);
            if ax < 0 || ay < 0 || ax >= a.w as i32 || ay >= a.h as i32 || x >= b.w as i32 || y >= b.h as i32 {
                return None;
            }
            if crosshair(b, x, y) || crosshair(a, ax, ay) {
                continue;
            }
            sum += f64::from((b.at(x, y) - a.at(ax, ay)).abs());
            n += 1;
        }
    }
    (n > 0).then(|| sum / f64::from(n))
}

/// The whole-pixel translation within `radius` that best matches `b` to `a` over `rect`
/// (searched on every second pixel, the nearest to none winning a tie) and the difference left
/// at it over every pixel.
pub fn best_shift(a: &Luma, b: &Luma, rect: Rect, radius: (i32, i32)) -> Option<((i32, i32), f64)> {
    let mut best: Option<((i32, i32), f64)> = None;
    for dy in -radius.1..=radius.1 {
        for dx in -radius.0..=radius.0 {
            let Some(d) = difference(a, b, rect, (dx, dy), 2) else { continue };
            let better = match best {
                None => true,
                Some(((bx, by), bd)) => d < bd || (d == bd && dx.abs() + dy.abs() < bx.abs() + by.abs()),
            };
            if better {
                best = Some(((dx, dy), d));
            }
        }
    }
    let (shift, _) = best?;
    Some((shift, difference(a, b, rect, shift, 1)?))
}

/// The brightness spread (standard deviation, levels) over `rect`.
pub fn contrast(l: &Luma, rect: Rect) -> f64 {
    let (x0, y0, bw, bh) = rect;
    let values: Vec<f64> = (y0..y0 + bh).flat_map(|y| (x0..x0 + bw).map(move |x| (x, y))).filter(|&(x, y)| x >= 0 && y >= 0 && (x as usize) < l.w && (y as usize) < l.h).map(|(x, y)| f64::from(l.at(x, y))).collect();
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64).sqrt()
}

/// What [`measure`] found.
#[derive(Debug, Clone, PartialEq)]
pub struct Measure {
    pub rect: Rect,
    /// A-B and A-C, mean absolute difference (levels).
    pub ab: f64,
    pub ac: f64,
    /// B = A translated by this (pixels; dy > 0 down the screen).
    pub shift: (i32, i32),
    /// A-B left at that translation.
    pub fit: f64,
    /// Each picture's contrast over the box.
    pub contrast: [f64; 3],
}

/// The measure of three pictures of one size; `Err` with why not.
pub fn measure(a: &Luma, b: &Luma, c: &Luma) -> Result<Measure, String> {
    if a.w != b.w || a.h != b.h || a.w != c.w || a.h != c.h {
        return Err(format!("the pictures differ in size ({}x{}, {}x{}, {}x{})", a.w, a.h, b.w, b.h, c.w, c.h));
    }
    let rect = centre_box(a.w, a.h);
    let ab = difference(a, b, rect, (0, 0), 1).ok_or("no pixel to compare")?;
    let ac = difference(a, c, rect, (0, 0), 1).ok_or("no pixel to compare")?;
    let (shift, fit) = best_shift(a, b, rect, search_radius(a.w, a.h)).ok_or("no translation to compare")?;
    Ok(Measure { rect, ab, ac, shift, fit, contrast: [contrast(a, rect), contrast(b, rect), contrast(c, rect)] })
}

/// The verdict on a [`Measure`] (module notes).
pub fn judge(m: &Measure) -> &'static str {
    if m.contrast.iter().any(|&c| c < FLAT_LEVELS) {
        return "flat";
    }
    let len = m.shift.0.abs().max(m.shift.1.abs());
    if len >= MOVED_PX && m.ab >= MOVED_LEVELS && m.ab >= MOVED_RATIO * m.ac && m.fit <= MOVED_FIT * m.ab {
        "moved"
    } else if m.ab <= STILL_LEVELS.max(STILL_RATIO * m.ac) || (len <= STILL_PX && m.ab <= STILL_SHARE * m.contrast[0]) {
        "still"
    } else {
        "unclear"
    }
}

/// The result line's words after the time and mode.
pub fn describe(m: &Measure) -> String {
    format!(
        "A-B {:.2}, A-C {:.2} levels over {}x{} px at the centre; B = A moved by ({:+}, {:+}) px (A-B {:.2} there); contrast {:.1}/{:.1}/{:.1}",
        m.ab, m.ac, m.rect.2, m.rect.3, m.shift.0, m.shift.1, m.fit, m.contrast[0], m.contrast[1], m.contrast[2]
    )
}

#[derive(Resource)]
struct Ab {
    queue: VecDeque<Spec>,
    run: Option<Run>,
    captured: [Option<Luma>; 3],
    dir: PathBuf,
}

struct Run {
    spec: Spec,
    phase: usize,
    since: f32,
    waiting: bool,
    unsettled: bool,
    /// The run's own switch value, put back after C (flip only).
    restore: bool,
}

const NAMES: [&str; 3] = ["a", "b", "c"];

/// The switch state of each shot in a flipped sequence.
const FLIP: [bool; 3] = [false, true, false];

/// Installed when the variable is set (`--play` only: the aids' clock).
pub fn install(app: &mut App) {
    let Ok(value) = std::env::var(ENV) else { return };
    match parse(&value) {
        Ok(specs) => {
            let listed: Vec<String> = specs.iter().map(|s| format!("{} s ({})", s.at, if s.hold { "hold" } else { "flip" })).collect();
            info!("test aid {ENV}: {} sequences of three shots: {}", specs.len(), listed.join(", "));
            let dir = std::env::var_os("NVF_PLAY_TEST_SHOT_DIR").map_or_else(|| PathBuf::from("."), PathBuf::from);
            app.insert_resource(Ab { queue: specs.into(), run: None, captured: [None, None, None], dir }).add_systems(Update, tick);
        }
        Err(e) => error!("test aid {ENV}: FAIL {e}"),
    }
}

fn set_switch(graphics: Option<&mut crate::graphics::Graphics>, on: bool, at: f32) {
    match graphics {
        Some(g) if g.terrain_parallax != on => {
            g.terrain_parallax = on;
            info!("terrain parallax ab at {at} s: switch {}", if on { "on" } else { "off" });
        }
        Some(_) => {}
        None => warn!("terrain parallax ab at {at} s: no graphics settings to switch"),
    }
}

fn tick(time: AidTime, mut ab: ResMut<Ab>, mut graphics: Option<ResMut<crate::graphics::Graphics>>, gate: Option<Res<crate::pipeline_gate::PipelineGate>>, mut commands: Commands) {
    let Some(now) = time.world_secs() else { return };
    let ab = &mut *ab;
    if ab.run.is_none() {
        let Some(&spec) = ab.queue.front().filter(|s| s.at <= now) else { return };
        ab.queue.pop_front();
        let restore = graphics.as_deref().is_some_and(|g| g.terrain_parallax);
        if !spec.hold {
            set_switch(graphics.as_deref_mut(), FLIP[0], spec.at);
        }
        ab.captured = [None, None, None];
        ab.run = Some(Run { spec, phase: 0, since: now, waiting: false, unsettled: false, restore });
        return;
    }
    let run = ab.run.as_mut().expect("a sequence runs");
    let at = run.spec.at;
    if run.waiting {
        if ab.captured[run.phase].is_none() {
            return;
        }
        run.waiting = false;
        run.phase += 1;
        run.since = now;
        if run.phase < 3 {
            if !run.spec.hold {
                set_switch(graphics.as_deref_mut(), FLIP[run.phase], at);
            }
            return;
        }
        // All three received.
        let (hold, unsettled, restore) = (run.spec.hold, run.unsettled, run.restore);
        ab.run = None;
        if !hold {
            set_switch(graphics.as_deref_mut(), restore, at);
        }
        let mode = if hold { "hold" } else { "flip" };
        let [Some(a), Some(b), Some(c)] = std::mem::take(&mut ab.captured) else { unreachable!("every shot received") };
        match measure(&a, &b, &c) {
            Ok(m) => {
                let verdict = if unsettled { "unsettled" } else { judge(&m) };
                info!("terrain parallax ab at {at} s ({mode}): {}; {verdict}", describe(&m));
            }
            Err(e) => warn!("terrain parallax ab at {at} s ({mode}): FAIL {e}"),
        }
        return;
    }
    let waited = now - run.since;
    let busy = gate.as_deref().map_or(0, |g| g.busy());
    if waited < SETTLE_SECS || (busy > 0 && waited < CAP_SECS) {
        return;
    }
    run.unsettled |= busy > 0;
    run.waiting = true;
    let phase = run.phase;
    let state = match (run.spec.hold, graphics.as_deref()) {
        (false, _) => if FLIP[phase] { "on" } else { "off" },
        (true, Some(g)) if g.terrain_parallax => "on (held)",
        (true, _) => "off (held)",
    };
    let path = ab.dir.join(format!("terrain_parallax_ab_{at}s_{}.png", NAMES[phase]));
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(dir);
    }
    info!("terrain parallax ab at {at} s: shot {} with the switch {state} (after {waited:.1} s, {busy} pipelines busy): {}", NAMES[phase], path.display());
    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path)).observe(move |ev: On<ScreenshotCaptured>, mut ab: ResMut<Ab>| {
        let size = ev.image.texture_descriptor.size;
        let luma = ev.image.data.as_deref().and_then(|d| Luma::from_pixels(d, size.width as usize, size.height as usize));
        if luma.is_none() {
            warn!("terrain parallax ab: shot {} not four bytes a pixel ({:?}); measured as empty", NAMES[phase], ev.image.texture_descriptor.format);
        }
        ab.captured[phase] = Some(luma.unwrap_or_default());
    });
}

#[cfg(test)]
mod tests;
