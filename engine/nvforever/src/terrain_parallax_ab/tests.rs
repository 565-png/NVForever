//! The terrain parallax readback's measure on generated pictures (no GPU).

use super::*;

const W: usize = 1280;
const H: usize = 720;

/// A textured picture: value noise on a 4-pixel lattice (aperiodic, deterministic), levels
/// 40..215, read at `(x, y)` through `map` (the picture's own coordinate to the texture's).
fn picture(map: impl Fn(f32, f32) -> (f32, f32)) -> Luma {
    let lattice = |i: i64, j: i64| -> f32 {
        let mut h = (i.wrapping_mul(73_856_093) ^ j.wrapping_mul(19_349_663)) as u64;
        h ^= h >> 13;
        h = h.wrapping_mul(0x5bd1_e995);
        h ^= h >> 15;
        (h % 1000) as f32 / 999.0
    };
    let noise = |x: f32, y: f32| -> f32 {
        let (gx, gy) = (x / 4.0, y / 4.0);
        let (i, j) = (gx.floor() as i64, gy.floor() as i64);
        let (fx, fy) = (gx - gx.floor(), gy - gy.floor());
        let top = lattice(i, j) * (1.0 - fx) + lattice(i + 1, j) * fx;
        let bottom = lattice(i, j + 1) * (1.0 - fx) + lattice(i + 1, j + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    };
    let mut v = Vec::with_capacity(W * H);
    for y in 0..H {
        for x in 0..W {
            let (tx, ty) = map(x as f32, y as f32);
            v.push(40.0 + 175.0 * noise(tx, ty));
        }
    }
    Luma { w: W, h: H, v }
}

fn plain() -> Luma {
    picture(|x, y| (x, y))
}

#[test]
fn the_times_and_modes_are_read() {
    assert_eq!(parse("20:hold, 8").unwrap(), [Spec { at: 8.0, hold: false }, Spec { at: 20.0, hold: true }], "in time order");
    assert_eq!(parse("8:flip").unwrap(), [Spec { at: 8.0, hold: false }]);
    for bad in ["", "x", "8:sideways", " , "] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn the_box_and_the_search_sit_at_the_centre_inside_the_picture() {
    assert_eq!(centre_box(W, H), (602, 331, 76, 57));
    assert_eq!(search_radius(W, H), (32, 96));
    // A small view: the search never leaves the picture.
    let (x0, y0, bw, bh) = centre_box(160, 90);
    let (rx, ry) = search_radius(160, 90);
    assert!(x0 - rx >= 0 && y0 - ry >= 0 && x0 + bw + rx <= 160 && y0 + bh + ry <= 90);
}

#[test]
fn pixels_of_four_bytes_are_read_as_the_channels_mean() {
    let l = Luma::from_pixels(&[30, 60, 90, 255, 0, 0, 0, 0], 2, 1).unwrap();
    assert_eq!(l.v, [60.0, 0.0]);
    assert!(Luma::from_pixels(&[0; 7], 2, 1).is_none(), "not four bytes a pixel");
    assert!(Luma::from_pixels(&[], 0, 0).is_none());
}

#[test]
fn a_translated_picture_is_measured_as_moved_by_its_translation() {
    let a = plain();
    // The texture moved 17 pixels down the screen: B(x, y) = A(x, y - 17).
    let b = picture(|x, y| (x, y - 17.0));
    let m = measure(&a, &b, &a).unwrap();
    assert_eq!(m.shift, (0, 17));
    assert!(m.fit < 1e-3, "the translation explains the whole change: {m:?}");
    assert!(m.ab > MOVED_LEVELS && m.ac == 0.0);
    assert_eq!(judge(&m), "moved");
    assert!(describe(&m).contains("B = A moved by (+0, +17) px"), "{}", describe(&m));
    // Up the screen and a little sideways: the sign is kept.
    let up = picture(|x, y| (x + 3.0, y + 9.0));
    let m = measure(&a, &up, &a).unwrap();
    assert_eq!((m.shift, judge(&m)), ((-3, -9), "moved"));
}

#[test]
fn an_unchanged_or_radially_changed_picture_is_still() {
    let a = plain();
    let m = measure(&a, &a, &a).unwrap();
    assert_eq!((m.ab, m.shift, judge(&m)), (0.0, (0, 0), "still"));
    // Head-on the shift grows from nothing at the centre outwards: no common translation.
    let (cx, cy) = (W as f32 / 2.0, H as f32 / 2.0);
    let radial = picture(|x, y| (cx + (x - cx) * 0.96, cy + (y - cy) * 0.96));
    let m = measure(&a, &radial, &a).unwrap();
    assert!(m.ab > 0.0 && m.ab < STILL_SHARE * m.contrast[0], "the picture did change, by a fraction of its grain: {m:?}");
    assert_eq!((m.shift, judge(&m)), ((0, 0), "still"), "{m:?}");
    // A change no larger than what changes anyway (A-C) is still, whatever the translation.
    let noisy = |seed: f32| picture(move |x, y| (x + 0.15 * (y * 0.37 + seed).sin(), y));
    let (b, c) = (noisy(1.0), noisy(2.0));
    let m = measure(&a, &b, &c).unwrap();
    assert!(m.ab <= STILL_RATIO * m.ac || m.ab <= STILL_LEVELS || m.shift.0.abs().max(m.shift.1.abs()) <= STILL_PX, "{m:?}");
    assert_eq!(judge(&m), "still");
}

#[test]
fn a_picture_without_texture_is_flat_and_a_wrong_size_fails() {
    let a = plain();
    let flat = Luma { w: W, h: H, v: vec![128.0; W * H] };
    assert_eq!(judge(&measure(&a, &flat, &a).unwrap()), "flat");
    assert!(measure(&a, &Luma::default(), &a).is_err(), "an unreadable shot");
}

#[test]
fn a_change_the_translation_does_not_explain_is_unclear() {
    let a = plain();
    // A different texture altogether: large difference, no translation fits it.
    let other = picture(|x, y| (x * 1.7 + 300.0, y * 1.3 + 200.0));
    let m = measure(&a, &other, &a).unwrap();
    assert!(m.ab > STILL_SHARE * m.contrast[0], "{m:?}");
    assert_eq!(judge(&m), "unclear", "{m:?}");
}

#[test]
fn the_crosshair_is_left_out() {
    let a = plain();
    // A crosshair drawn on B alone changes nothing measured.
    let mut b = a.clone();
    for y in H / 2 - 8..H / 2 + 8 {
        for x in W / 2 - 8..W / 2 + 8 {
            b.v[y * W + x] = 255.0;
        }
    }
    let m = measure(&a, &b, &a).unwrap();
    assert_eq!((m.ab, m.shift), (0.0, (0, 0)));
}
