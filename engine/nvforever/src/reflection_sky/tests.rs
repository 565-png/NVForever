use super::*;

fn near(a: Vec3, b: Vec3, tol: f32) -> bool {
    (a - b).abs().max_element() <= tol
}

/// A clear day: three distinct dome colours (linear), an ambient, the light 45 degrees up
/// toward +X.
fn day() -> Inputs {
    Inputs::Outdoors {
        horizon: Vec3::new(0.6, 0.65, 0.7),
        lower: Vec3::new(0.3, 0.45, 0.8),
        upper: Vec3::new(0.05, 0.1, 0.6),
        ambient: Vec3::new(0.5, 0.5, 0.55),
        sun: Vec3::new(0.7, 0.65, 0.6),
        sun_dir: Vec3::new(1.0, 1.0, 0.0).normalize(),
    }
}

fn dark(inputs: Inputs) -> Inputs {
    match inputs {
        Inputs::Outdoors { horizon, lower, upper, ambient, sun_dir, .. } => Inputs::Outdoors { horizon, lower, upper, ambient, sun: Vec3::ZERO, sun_dir },
        other => other,
    }
}

fn room() -> Inputs {
    Inputs::Indoors { cell: Some(0x1234), ambient: Vec3::new(0.4, 0.35, 0.3), fog: Vec3::new(0.2, 0.25, 0.3) }
}

/// The units identity of nvf_assets `ReflectionCube` ("Units"): with the cube's intensity
/// `light_unit()` and the default exposure E, a texel made from the view's exposed ambient A
/// reflects `texel x intensity x E = A x max(A, 1e-6)^1.2`, about A^2.2.
#[test]
fn a_texel_from_the_ambient_reflects_the_converted_ambient() {
    let exposure = bevy::camera::Exposure::default().exposure();
    for a in [0.0f32, 1e-7, 0.01, 0.2, 0.54, 0.9, 1.0] {
        let reflected = texel(Vec3::splat(a)).x * reflection_cube::light_unit() * exposure;
        let want = a * a.max(1e-6).powf(1.2);
        assert!((reflected - want).abs() <= 1e-6 * want.max(1.0), "A = {a}: {reflected} against {want}");
        if a >= 0.01 {
            assert!((reflected / a.powf(2.2) - 1.0).abs() < 1e-4, "A = {a}: about A^2.2");
        }
    }
    assert_eq!(texel(Vec3::new(-0.5, 0.0, 0.25)).x, 0.0, "held at 0");
}

/// Indoors the zenith is the ambient colour itself (the fog band is gone there): the
/// ambient-matched case, texel = A^2.2 in the identity's terms (A the display value times the
/// interior ambient share, as the ambient light's brightness carries it).
#[test]
fn an_interior_zenith_is_the_ambient_texel() {
    let Inputs::Indoors { ambient, .. } = room() else { unreachable!() };
    let c = SkyContent::new(&room(), &DomeProfile::fallback());
    assert!(near(c.colour(Vec3::Y, 0), texel(ambient * crate::sky::INTERIOR_AMBIENT_SHARE), 1e-5));
}

/// Outdoors a direction whose dome colour equals the ambient colour gives the ambient's
/// texel: the dome's linear colours go through the same display conversion and share as the
/// ambient light (sky.rs update_sky), not a second decode.
#[test]
fn an_outdoor_sky_of_the_ambient_colour_is_the_ambient_texel() {
    let ambient_linear = Vec3::new(0.25, 0.3, 0.35);
    let inputs = Inputs::Outdoors {
        horizon: ambient_linear,
        lower: ambient_linear,
        upper: ambient_linear,
        ambient: ambient_linear.map(crate::lights::linear_to_display),
        sun: Vec3::ZERO,
        sun_dir: Vec3::Y,
    };
    let c = SkyContent::new(&inputs, &DomeProfile::fallback());
    let want = texel(ambient_linear.map(crate::lights::linear_to_display) * crate::sky::AMBIENT_SHARE);
    for e in [0.3f32, 0.7, 1.0] {
        let d = Vec3::new((1.0 - e * e).sqrt(), e, 0.0);
        assert!(near(c.colour(d, 0), want, 1e-4), "{e}: {:?} against {want:?}", c.colour(d, 0));
    }
}

/// Zenith, horizon and nadir at level 0 (no light): the zenith the dome's top colour, the
/// nadir the darkened ambient, the horizon halfway between the ground and the dome's lowest
/// colour.
#[test]
fn zenith_horizon_and_nadir() {
    let inputs = dark(day());
    let Inputs::Outdoors { horizon, lower, upper, ambient, .. } = inputs.clone() else { unreachable!() };
    let profile = DomeProfile::fallback();
    let c = SkyContent::new(&inputs, &profile);
    let share = crate::sky::AMBIENT_SHARE;
    let dome = |e: f32| texel(Vec3::from_array(crate::sky::dome_color(profile.weights(e), lower.to_array(), horizon.to_array(), upper.to_array())).map(crate::lights::linear_to_display) * share);
    let ground = texel(ambient * share * GROUND_FACTOR);
    assert!(near(c.colour(Vec3::Y, 0), dome(1.0), 1e-4), "zenith");
    assert!(near(c.colour(Vec3::NEG_Y, 0), ground, 1e-6), "nadir");
    assert!(near(c.colour(Vec3::X, 0), (dome(0.0) + ground) * 0.5, 1e-4), "horizon");
    // The fallback's zenith is sky-upper alone.
    assert!(near(dome(1.0), texel(upper.map(crate::lights::linear_to_display) * share), 1e-4));
}

/// The lobe peaks toward the directional light: brighter there than a few degrees off, at
/// every level, and wider (less peaked) at each level.
#[test]
fn the_lobe_peaks_toward_the_light_and_widens() {
    let inputs = day();
    let Inputs::Outdoors { sun_dir, .. } = inputs else { unreachable!() };
    let lit = SkyContent::new(&inputs, &DomeProfile::fallback());
    let unlit = SkyContent::new(&dark(day()), &DomeProfile::fallback());
    let off = (Quat::from_rotation_z(0.15) * sun_dir).normalize();
    let mut ratios = Vec::new();
    for level in 0..SPECULAR_LEVELS {
        let peak = lit.colour(sun_dir, level) - unlit.colour(sun_dir, level);
        let beside = lit.colour(off, level) - unlit.colour(off, level);
        assert!(peak.x > beside.x && peak.x > 0.0, "level {level}");
        ratios.push(beside.x / peak.x);
    }
    assert!(ratios.windows(2).all(|w| w[1] > w[0]), "wider at each level: {ratios:?}");
    // Its total is kept: the peak falls as it widens.
    let peak = |level| (lit.colour(sun_dir, level) - unlit.colour(sun_dir, level)).x;
    assert!(peak(SPECULAR_LEVELS - 1) < peak(0));
}

/// The levels approach each hemisphere's mean: the distance from the mean falls with the
/// level, and the last level is the mean (zenith and nadir, no light).
#[test]
fn levels_approach_the_hemisphere_means() {
    for inputs in [dark(day()), room()] {
        let c = SkyContent::new(&inputs, &DomeProfile::fallback());
        let (up, down) = c.means();
        for (d, mean) in [(Vec3::Y, up), (Vec3::NEG_Y, down), (Vec3::new(0.0, 0.8, 0.6), up)] {
            let dist: Vec<f32> = (0..SPECULAR_LEVELS).map(|l| (c.colour(d, l) - mean).length()).collect();
            assert!(dist.windows(2).all(|w| w[1] <= w[0] + 1e-6), "{inputs:?} {d}: {dist:?}");
            assert!(dist[SPECULAR_LEVELS as usize - 1] < 1e-4, "{inputs:?} {d}: the last level is the mean");
        }
    }
}

/// Indoors the fog colour holds the horizon and fades by the zenith and the nadir.
#[test]
fn an_interior_has_a_fog_band_at_the_horizon() {
    let Inputs::Indoors { fog, ambient, .. } = room() else { unreachable!() };
    let c = SkyContent::new(&room(), &DomeProfile::fallback());
    let fog_t = texel(fog);
    assert!(near(c.colour(Vec3::X, 0), fog_t, 1e-5), "the horizon is the fog colour");
    assert!(near(c.colour(Vec3::NEG_Y, 0), texel(ambient * GROUND_FACTOR), 1e-5), "the nadir the darker ground");
    assert!((c.colour(Vec3::new(0.0, 0.2, 0.98).normalize(), 0) - fog_t).length() < (c.colour(Vec3::Y, 0) - fog_t).length());
}

/// A change below a step does not refill; one above does (module notes, "When").
#[test]
fn small_changes_do_not_refill_and_large_ones_do() {
    let base = day();
    let with = |f: &dyn Fn(&mut Vec3, &mut Vec3, &mut Vec3)| {
        let mut i = base.clone();
        if let Inputs::Outdoors { upper, ambient, sun_dir, .. } = &mut i {
            f(upper, ambient, sun_dir);
        }
        i
    };
    assert!(!base.moved(&base));
    // The ambient (display value) by half a step, then by two.
    assert!(!with(&|_, a, _| a.x += COLOUR_STEP * 0.5).moved(&base));
    assert!(with(&|_, a, _| a.x += COLOUR_STEP * 2.0).moved(&base));
    // A dome colour: compared in display value, so a linear change a little under the step's
    // worth does not count and a larger one does.
    assert!(!with(&|u, _, _| u.z += 0.002).moved(&base));
    assert!(with(&|u, _, _| u.z += 0.03).moved(&base));
    // The light's direction by one degree, then by five.
    let turn = |deg: f32| move |_: &mut Vec3, _: &mut Vec3, d: &mut Vec3| *d = Quat::from_rotation_y(deg.to_radians()) * *d;
    assert!(!with(&turn(1.0)).moved(&base));
    assert!(with(&turn(5.0)).moved(&base));
    // Without a light its direction does not matter.
    assert!(!dark(with(&turn(30.0))).moved(&dark(base.clone())));
    // Indoors against outdoors, and another cell.
    assert!(room().moved(&base) && base.moved(&room()));
    let other = Inputs::Indoors { cell: Some(0x9999), ambient: Vec3::new(0.4, 0.35, 0.3), fog: Vec3::new(0.2, 0.25, 0.3) };
    assert!(other.moved(&room()));
    assert!(!room().moved(&room()));
}

/// The attach conditions: both rows on, an authored set in the mount, the gate installed and
/// the world camera present; the mount is not asked while a row is off.
#[test]
fn the_cube_is_made_only_when_every_condition_holds() {
    let installed = GateState::Installed;
    assert_eq!(decide(true, true, || true, installed, true), Decision::Allocate);
    assert_eq!(decide(false, true, || panic!("not asked"), installed, true), Decision::Off("setting"));
    assert_eq!(decide(true, false, || panic!("not asked"), installed, true), Decision::Off("Authored materials off"));
    assert_eq!(decide(true, true, || false, installed, true), Decision::Off("no authored sets in the mount"));
    assert_eq!(decide(true, true, || true, GateState::Failed, true), Decision::Off("reflection gate not installed"));
    assert_eq!(decide(true, true, || true, GateState::Pending, true), Decision::Wait);
    assert_eq!(decide(true, true, || true, installed, false), Decision::Wait);
}

/// The dome profile from vertices: each step the mean of its vertices, empty steps filled
/// between their neighbours, vertices below the horizon left out; no vertex above it gives
/// the fallback.
#[test]
fn the_dome_profile_follows_the_vertices() {
    let at = |e: f32| [(1.0 - e * e).sqrt() * 500.0, 0.0, e * 500.0];
    let positions = [at(0.01), at(0.01), at(0.99), at(-0.5)];
    let colours = [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0]];
    let p = DomeProfile::from_vertices(&positions, &colours);
    assert_eq!(p.0[0], [0.5, 0.5, 0.0]);
    assert_eq!(p.0[DOME_PROFILE_BINS - 1], [0.0, 0.0, 1.0]);
    let mid = p.0[DOME_PROFILE_BINS / 2];
    assert!(mid[2] > 0.4 && mid[2] < 0.6 && (mid.iter().sum::<f32>() - 1.0).abs() < 1e-5, "{mid:?}");
    let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
    assert!(close(p.weights(0.0), [0.5, 0.5, 0.0]));
    assert!(close(p.weights(1.0), [0.0, 0.0, 1.0]));
    assert_eq!(DomeProfile::from_vertices(&[at(-0.3)], &[[1.0; 4]]), DomeProfile::fallback());
    let f = DomeProfile::fallback();
    assert!(f.weights(1.0)[2] > 0.99 && f.weights(0.0)[1] > 0.7);
}

/// The summary names the reason when off and the counts when on.
#[test]
fn the_summary_line() {
    let mut s = ReflectionSky::new(false, true);
    s.phase = Phase::Off("setting");
    assert_eq!(s.line(), "off (setting)");
    s.phase = Phase::On;
    s.fills_outdoors = 3;
    s.fills_indoors = 1;
    s.unchanged = 40;
    assert_eq!(s.line(), "4 fills (3 outdoors, 1 indoors), 40 checks skipped as unchanged");
}
