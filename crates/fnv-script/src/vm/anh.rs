//! The Anh NVSE plugin's pure functions: array extremes, angles, vector3 and quaternion
//! arrays, string padding [documented: each page in the wiki export; docs/spec-notes/
//! ext-anh.md]. Probed functions carry `[observed]` citations from Fallout: New Vegas
//! 1.4.0.525 + xNVSE 6.4.9 + AnhNVSE 131; unprobed details remain `[hypothesis]` or
//! `[design]`.

use super::{ArrayKind, Vm, VmError, XKey, XVal};
use crate::ir::XCall;

type V3 = [f64; 3];
type Quat = [f64; 4];

/// The Hamilton product a·b (w, x, y, z).
fn qmul(a: Quat, b: Quat) -> Quat {
    let [w1, x1, y1, z1] = a;
    let [w2, x2, y2, z2] = b;
    [
        w1 * w2 - x1 * x2 - y1 * y2 - z1 * z2,
        w1 * x2 + x1 * w2 + y1 * z2 - z1 * y2,
        w1 * y2 - x1 * z2 + y1 * w2 + z1 * x2,
        w1 * z2 + x1 * y2 - y1 * x2 + z1 * w2,
    ]
}

fn unit<const N: usize>(v: [f64; N]) -> [f64; N] {
    let n = v.iter().map(|c| c * c).sum::<f64>().sqrt();
    if n == 0.0 { v } else { v.map(|c| c / n) }
}

/// DirectInput keyboard scan code to key name for `ScancodeToChar`: uppercase letters
/// and full key names (`1 => "Escape"`, `18 => "E"`, `30 => "A"`, `59 => "F1"`)
/// [observed: Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + AnhNVSE 131, probe
/// anh_03_scancode_to_char_layout; other keyboard codes 2..=220 reuse `KEY_NAMES`
/// and unmapped codes return "" [hypothesis]].
fn scancode_name(code: i64) -> &'static str {
    if code == 1 {
        return "Escape";
    }
    let Ok(c) = u16::try_from(code) else { return "" };
    if c >= 255 {
        return "";
    }
    crate::keys::KEY_NAMES.iter().find(|(k, _)| *k == c).map_or("", |(_, name)| *name)
}

impl Vm<'_, '_> {
    /// An array's numbers in order (non-numbers left out); `None` for no array.
    fn numbers_of(&mut self, v: &XVal) -> Result<Option<Vec<f64>>, VmError> {
        let XVal::Array(id) = v else { return Ok(None) };
        Ok(self.store()?.array(*id).map(|a| a.entries.iter().filter_map(|(_, e)| if let XVal::Num(x) = e { Some(*x) } else { None }).collect()))
    }

    fn number_list(&mut self, xs: &[f64]) -> Result<XVal, VmError> {
        let es = xs.iter().enumerate().map(|(i, &x)| (XKey::Num(i as f64), XVal::Num(x))).collect();
        Ok(XVal::Array(self.store()?.new_array(ArrayKind::List, es)))
    }

    /// An Anh function by lower-case name; `None` for a name this module does not build.
    pub(super) fn anh_function(&mut self, n: &str, c: &XCall) -> Result<Option<XVal>, VmError> {
        let mut args = Vec::with_capacity(c.args.len());
        for a in &c.args {
            args.push(self.xeval(a)?);
        }
        let num = |k: usize| args.get(k).and_then(|v| v.num().ok()).unwrap_or(0.0);
        let text = |k: usize| match args.get(k) {
            Some(XVal::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let v3 = |vm: &mut Self, k: usize| -> Result<V3, VmError> {
            let xs = match args.get(k) {
                Some(v) => vm.numbers_of(v)?.unwrap_or_default(),
                None => Vec::new(),
            };
            Ok(std::array::from_fn(|i| xs.get(i).copied().unwrap_or(0.0)))
        };
        let quat = |vm: &mut Self, k: usize| -> Result<Quat, VmError> {
            let xs = match args.get(k) {
                Some(v) => vm.numbers_of(v)?.unwrap_or_default(),
                None => Vec::new(),
            };
            Ok(std::array::from_fn(|i| xs.get(i).copied().unwrap_or(0.0)))
        };
        Ok(Some(match n {
            // The largest, the smallest and their difference of an array's numbers
            // [documented: Ar_Max, Ar_Min, Ar_RangeMath; observed: Fallout: New Vegas
            // 1.4.0.525 + xNVSE 6.4.9 + AnhNVSE 131, probe anh_01_ar_max_min_range_empty:
            // [6, 2, 10, 7] gives 10.00, 2.00, 8.00; empty or string-only arrays crash
            // AnhNVSE 131 with 0xC0000005]; NVForever safely returns 0 [design].
            "ar_max" | "ar_min" | "ar_rangemath" => {
                let xs = match args.first() {
                    Some(v) => self.numbers_of(v)?.unwrap_or_default(),
                    None => Vec::new(),
                };
                let (lo, hi) = (xs.iter().copied().fold(f64::INFINITY, f64::min), xs.iter().copied().fold(f64::NEG_INFINITY, f64::max));
                XVal::Num(match (n, xs.is_empty()) {
                    (_, true) => 0.0,
                    ("ar_max", _) => hi,
                    ("ar_min", _) => lo,
                    _ => hi - lo,
                })
            }
            // [documented: DegToRad; RadToDeg].
            "degtorad" => XVal::Num(num(0).to_radians()),
            "radtodeg" => XVal::Num(num(0).to_degrees()),
            // The quadrant 1-4 of an angle in degrees, truncated to integer degrees modulo
            // 360 [observed: Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + AnhNVSE 131,
            // probe anh_02_angle_quadrant_edges: 0, 45, 1.5708, 3.1416, 4.7124, -0.5 -> 1;
            // 90, 135 -> 2; 180, 225 -> 3; 270, 315, -45 -> 4].
            "getanglequadrant" => {
                let a = (num(0) as i64).rem_euclid(360);
                XVal::Num((a / 90 + 1) as f64)
            }
            // [documented: V3Mult, (1,2,3)·(4,5,6) gives (4,10,18); V3MultByScalar, (1,2,3)
            // by 2 gives (2,4,6); V3Dotproduct; V3NormalizeAlt; V3CrossproductAlt, "the
            // crossproduct of vectorA and vectorB" (A×B); V3Lerp, t from 0 to 1].
            "v3mult" => {
                let (a, b) = (v3(self, 0)?, v3(self, 1)?);
                return self.number_list(&[a[0] * b[0], a[1] * b[1], a[2] * b[2]]).map(Some);
            }
            "v3multbyscalar" => {
                let (a, s) = (v3(self, 0)?, num(1));
                return self.number_list(&a.map(|x| x * s)).map(Some);
            }
            "v3dotproduct" => {
                let (a, b) = (v3(self, 0)?, v3(self, 1)?);
                XVal::Num(a[0] * b[0] + a[1] * b[1] + a[2] * b[2])
            }
            "v3normalizealt" => {
                let a = v3(self, 0)?;
                return self.number_list(&unit(a)).map(Some);
            }
            "v3crossproductalt" => {
                let (a, b) = (v3(self, 0)?, v3(self, 1)?);
                return self.number_list(&[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]).map(Some);
            }
            // Vector3 linear interpolation a + (b - a) * t without clamping t [observed:
            // Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + AnhNVSE 131, probes
            // anh_04_v3lerp_arrays and anh_10_v3lerp_extrapolate: [0,0,0]..[10,20,30] at
            // 0.5 -> [5,10,15]; t=2 -> 20, t=-1 -> -10].
            "v3lerp" => {
                let (a, b, t) = (v3(self, 0)?, v3(self, 1)?, num(2));
                return self.number_list(&std::array::from_fn::<f64, 3, _>(|i| a[i] + (b[i] - a[i]) * t)).map(Some);
            }
            // The quadratic Bézier curve through its three points at t [documented:
            // GenerateBezierPoint, "created by 3 points", t from 0 to 1].
            "generatebezierpoint" => {
                let (p0, p1, p2, t) = (v3(self, 0)?, v3(self, 1)?, v3(self, 2)?, num(3));
                let u = 1.0 - t;
                return self.number_list(&std::array::from_fn::<f64, 3, _>(|i| u * u * p0[i] + 2.0 * u * t * p1[i] + t * t * p2[i])).map(Some);
            }
            // Quaternions as [w, x, y, z] [documented: QNormalizeAlt "([qW, qX, qY, qZ])";
            // observed: anh_09_q_axis_angle_zero_order, anh_05_q_axis_angle_and_mult_vec3].
            "qnormalizealt" => {
                let q = quat(self, 0)?;
                return self.number_list(&unit(q)).map(Some);
            }
            // A·B [documented: QMultQuatQuatAlt, "the product of quatA and quatB"].
            "qmultquatquatalt" => {
                let (a, b) = (quat(self, 0)?, quat(self, 1)?);
                return self.number_list(&qmul(a, b)).map(Some);
            }
            // Returns a 3-element vector3 [observed: anh_05_q_axis_angle_and_mult_vec3];
            // rotation convention q·v·q⁻¹ remains [hypothesis] (anh_05 observed
            // rot=(1.000,0.000,0.000) for q=(0.707,0,0,0.707) on [1,0,0]).
            "qmultquatvector3alt" => {
                let (q, v) = (unit(quat(self, 0)?), v3(self, 1)?);
                let conj = [q[0], -q[1], -q[2], -q[3]];
                let r = qmul(qmul(q, [0.0, v[0], v[1], v[2]]), conj);
                return self.number_list(&[r[1], r[2], r[3]]).map(Some);
            }
            // `axis angle`: angle in degrees, returning [w, x, y, z] [observed: Fallout:
            // New Vegas 1.4.0.525 + xNVSE 6.4.9 + AnhNVSE 131, probes
            // anh_09_q_axis_angle_zero_order (0 -> [1,0,0,0]) and
            // anh_05_q_axis_angle_and_mult_vec3 (90 around [0,0,1] -> [0.707,0,0,0.707])].
            "qfromaxisanglealt" => {
                let (axis, h) = (unit(v3(self, 0)?), num(1).to_radians() / 2.0);
                return self.number_list(&[h.cos(), axis[0] * h.sin(), axis[1] * h.sin(), axis[2] * h.sin()]).map(Some);
            }
            // `a b ratio [spherical]`: normalised linear by default, spherical with the
            // fourth argument [documented: QInterpolateAlt]; the shorter way round
            // [hypothesis].
            "qinterpolatealt" => {
                let (a, mut b, t, slerp) = (unit(quat(self, 0)?), unit(quat(self, 1)?), num(2), num(3) != 0.0);
                let mut d: f64 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
                if d < 0.0 {
                    b = b.map(|v| -v);
                    d = -d;
                }
                let out: Quat = if !slerp || d > 0.9995 {
                    unit(std::array::from_fn(|i| a[i] + t * (b[i] - a[i])))
                } else {
                    let th = d.acos();
                    let (sa, sb) = (((1.0 - t) * th).sin() / th.sin(), (t * th).sin() / th.sin());
                    std::array::from_fn(|i| sa * a[i] + sb * b[i])
                };
                return self.number_list(&out).map(Some);
            }
            // [documented: Sv_Capitalize, "pepeglare" gives "Pepeglare"; only the first
            // character].
            "sv_capitalize" => {
                let s = text(0);
                let mut cs = s.chars();
                XVal::Str(match cs.next() {
                    Some(f) => f.to_uppercase().chain(cs).collect(),
                    None => s,
                })
            }
            // `text length pad`: the pad repeated (cut at the end) before or after the
            // text until it is `length` long; a longer text or an empty pad as it is
            // [documented: Sv_PadStart; Sv_PadEnd].
            "sv_padstart" | "sv_padend" => {
                let (s, len, pad) = (text(0), num(1).max(0.0) as usize, text(2));
                let have = s.chars().count();
                if have >= len || pad.is_empty() {
                    XVal::Str(s)
                } else {
                    let fill: String = pad.chars().cycle().take(len - have).collect();
                    XVal::Str(if n == "sv_padstart" { fill + &s } else { s + &fill })
                }
            }
            // [documented: ScancodeToChar; observed: Fallout: New Vegas 1.4.0.525 +
            // xNVSE 6.4.9 + AnhNVSE 131, probe anh_03_scancode_to_char_layout:
            // 1="Escape", 59="F1", 18="E", 30="A"; other keyboard codes / "" fallback
            // [hypothesis]].
            "scancodetochar" => XVal::Str(scancode_name(num(0) as i64).to_string()),
            // `array`: the array as a text [documented: ArrayToString, "Returns the
            // specified array variable as a string"]; the form is PrintVar's (elements in
            // brackets, a map's as `key: value`) [decision: the page shows no example];
            // "" for no array [hypothesis].
            "arraytostring" => match args.first() {
                Some(XVal::Array(id)) => XVal::Str(self.brief_array(*id)?),
                _ => XVal::Str(String::new()),
            },
            _ => return Ok(None),
        }))
    }
}
