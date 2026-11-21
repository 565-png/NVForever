//! What the crosshair points at: the collision ray's hit, or a reference's render bounds
//! when the ray finds no reference (pure, testable without Bevy's app).
//!
//! The rule [hypothesis, docs/spec-notes/simulation.md §5 "Crosshair targeting"]: the
//! target is the activatable reference nearest along the view within [`REACH`] of the eye.
//! Candidates are the reference whose collision the ray hits first, and references whose
//! render bounds (a mesh's box in its own space, i.e. an oriented box in the world) the
//! ray enters before the first collision hit of any kind (walls, floors and tables hide
//! what is behind them). Many small items have no collision, so the bounds are what make
//! them pickable. A box the eye is inside does not count, nor one that encloses another
//! candidate's along the ray (a bed's around the suitcase lying on it,
//! [`set_aside_enclosing`]).

use bevy::math::{Affine3A, Vec3};

/// How far from the eye the player can activate, game units [hypothesis: a placeholder of
/// about two metres at ~70 units per metre; no game setting in the masters names an
/// activation distance (simulation.md §5). In game the setting `iActivatePickLength`
/// reads 150 [observed: capture mv_trial, physics.md §6.3]; that it is this distance is
/// [documented: GECK wiki GetCrosshairRef] but unmeasured (job mv_values)].
pub const REACH: f32 = 150.0;

/// The first collision hit along the view (any owner), from `fnv_phys`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    /// The placed reference hit, when the collider belongs to one.
    pub form_id: Option<u32>,
    pub distance: f32,
}

/// How far behind the surface the ray first hits a door may stand and still be the
/// collision hit, game units. Some buildings' collision has a face a few units in front
/// of their door: the Nipton town hall's mesh lies 8.5 units before its door's slab
/// (0x00130CFF), so the door was never targeted (#1339) [observed 2026-11-01; design:
/// the reach].
pub const DOOR_SLACK: f32 = 16.0;

/// The collision hit the target is chosen from: the first hit, or a door found within
/// [`DOOR_SLACK`] behind it when the first hit is not something the player can
/// activate (`activatable`) and the door is.
pub fn door_behind(first: Option<RayHit>, behind: Option<RayHit>, mut activatable: impl FnMut(u32) -> bool) -> Option<RayHit> {
    match (first, behind) {
        (Some(f), Some(b)) if b.distance <= f.distance + DOOR_SLACK && !f.form_id.is_some_and(&mut activatable) && b.form_id.is_some_and(&mut activatable) => Some(b),
        (f, _) => f,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Collision,
    Bounds,
    /// The ray picked a bed someone lies on, and the target is the sleeper
    /// ([`to_occupant`]); the distance is to the sleeper's body.
    Occupant,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pick {
    pub form_id: u32,
    /// Distance from the eye along the view, game units.
    pub distance: f32,
    pub source: Source,
}

/// Choose the target from the collision hit and the render-bounds hits (reference,
/// entry distance). `accept` says whether a reference can be activated now (present in
/// the simulation, of an activatable type); it is asked only for candidates that would
/// win. Ties go to the collision hit.
pub fn choose(
    hit: Option<RayHit>,
    bounds: impl IntoIterator<Item = (u32, f32)>,
    reach: f32,
    mut accept: impl FnMut(u32) -> bool,
) -> Option<Pick> {
    let limit = hit.map_or(reach, |h| h.distance.min(reach));
    let mut best = hit
        .and_then(|h| Some((h.form_id?, h.distance)))
        .filter(|&(id, d)| d <= reach && accept(id))
        .map(|(form_id, distance)| Pick { form_id, distance, source: Source::Collision });
    for (id, d) in bounds {
        if !(0.0..=limit).contains(&d) {
            continue;
        }
        match best {
            Some(b) if b.distance <= d => {}
            Some(b) if b.form_id == id => best = Some(Pick { distance: d, ..b }),
            _ if accept(id) => best = Some(Pick { form_id: id, distance: d, source: Source::Bounds }),
            _ => {}
        }
    }
    best
}

/// [`choose`] for the crosshair, with some boxes only rough (`rough`: an actor's render
/// box, its skinned mesh's bind-pose bounds, which do not follow a body lying on the
/// floor [hypothesis]). A precise box the ray enters before the collision hit wins over
/// any rough one, nearer or not: an ammo box or a syringe beside a body is taken, not the
/// body searched (#3125). Without one, the rule is [`choose`]'s over every box, so a body
/// is still searched by its box and an actor's collision hit keeps winning.
pub fn choose_precise_first(
    hit: Option<RayHit>,
    bounds: impl IntoIterator<Item = (u32, f32, bool)>,
    reach: f32,
    mut accept: impl FnMut(u32) -> bool,
) -> Option<Pick> {
    let all: Vec<(u32, f32, bool)> = bounds.into_iter().collect();
    let precise = choose(hit, all.iter().filter(|b| !b.2).map(|b| (b.0, b.1)), reach, &mut accept);
    if precise.is_some_and(|p| p.source == Source::Bounds) {
        return precise;
    }
    choose(hit, all.iter().map(|b| (b.0, b.1)), reach, accept)
}

/// Render boxes (reference, entry, exit, rough) without those of furniture (`furniture`)
/// that hold, along the ray, the whole stretch through another reference's precise box
/// (not an actor's rough one, [`choose_precise_first`]) that is itself a candidate
/// (entered before `hit`, the first collision hit as the ray met it, within `reach`) and
/// can be activated (`accept`): the furniture's box set aside, the thing lying in it
/// stays [design, #3323]. A bed's box runs from the floor to the top of its headboard and
/// encloses what lies on the mattress, so the ray enters it before a suitcase lying there
/// and the bed, nearest, won though the suitcase was what the crosshair was on. Only a
/// furniture's box is set aside: another activator's (a switch panel) keeps priority over
/// a smaller one behind it. Never a collision hit: the mattress still picks the bed, and
/// a wall in front still hides the suitcase. `hit` is the raw first hit, not a door
/// promoted from behind it (`door_behind`), so nothing behind the wall sets a box aside.
/// Two boxes over the same stretch both stay; a sleeper's rough box sets no bed's aside,
/// so [`to_occupant`] still sees the bed. The result is [`choose_precise_first`]'s input.
pub fn set_aside_enclosing(
    hit: Option<RayHit>,
    boxes: &[(u32, f32, f32, bool)],
    reach: f32,
    furniture: impl Fn(u32) -> bool,
    mut accept: impl FnMut(u32) -> bool,
) -> Vec<(u32, f32, bool)> {
    let limit = hit.map_or(reach, |h| h.distance.min(reach));
    let mut kept = Vec::with_capacity(boxes.len());
    for &(id, enter, exit, rough) in boxes {
        let holds = |&&(other, e, x, r): &&(u32, f32, f32, bool)| other != id && !r && (0.0..=limit).contains(&e) && enter <= e && x <= exit && (enter < e || x < exit);
        if !furniture(id) || !boxes.iter().filter(holds).any(|b| accept(b.0)) {
            kept.push((id, enter, rough));
        }
    }
    kept
}

/// A bed an actor lies on (or is getting onto or off) yields the crosshair to that actor
/// when the actor's body is within `reach` of the eye [design, #3231: what the original
/// targets is not captured]. The ray enters the bed's box, which encloses the mattress,
/// before the sleeper's, and the sleeper's render box is rough (its bind-pose bounds, not
/// the lying body), so without this the bed was the target, refused as in use, and the
/// sleeper could not be woken and talked to. A bed is one user per reference
/// (docs/spec-notes/furniture.md §5), so nothing the player could use is lost.
/// `occupant` names the actor on a bed reference (not the player) and how far its body is
/// from the eye; a body beyond `reach` (the far end of a wide bed, aimed at its foot)
/// leaves the bed the target. `accept` is [`choose`]'s; a sleeper that cannot be
/// activated now leaves the bed too. The pick's distance becomes the body's.
pub fn to_occupant(picked: Option<Pick>, occupant: impl FnOnce(u32) -> Option<(u32, f32)>, reach: f32, accept: impl FnOnce(u32) -> bool) -> Option<Pick> {
    let p = picked?;
    match occupant(p.form_id) {
        Some((actor, d)) if actor != p.form_id && (0.0..=reach).contains(&d) && accept(actor) => Some(Pick { form_id: actor, distance: d, source: Source::Occupant }),
        _ => Some(p),
    }
}

/// Distance from `p` to the segment `a`–`b` (a posed bone).
pub fn segment_distance(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 > 1e-12 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

/// Distance from `p` (world space) to the nearest point of a box given in an entity's own
/// space (`center`, `half`) placed by `to_world`; 0 inside. Exact for a uniform scale.
/// `None` for a degenerate transform.
pub fn box_distance(p: Vec3, to_world: &Affine3A, center: Vec3, half: Vec3) -> Option<f32> {
    if to_world.matrix3.determinant().abs() < 1e-12 {
        return None;
    }
    let local = to_world.inverse().transform_point3(p) - center;
    let near = center + local.clamp(-half, half);
    Some(p.distance(to_world.transform_point3(near)))
}

/// Distance along a ray (`origin`, unit `dir`, world space) at which it enters a box
/// given in an entity's own space (`center`, `half` extents) placed by `to_world`. `None`
/// when the ray misses, the box is behind, or `origin` is inside the box.
pub fn ray_box(origin: Vec3, dir: Vec3, to_world: &Affine3A, center: Vec3, half: Vec3) -> Option<f32> {
    ray_box_span(origin, dir, to_world, center, half).map(|s| s.0)
}

/// [`ray_box`]'s entry distance and the distance at which the ray leaves the box.
pub fn ray_box_span(origin: Vec3, dir: Vec3, to_world: &Affine3A, center: Vec3, half: Vec3) -> Option<(f32, f32)> {
    if to_world.matrix3.determinant().abs() < 1e-12 {
        return None;
    }
    let inv = to_world.inverse();
    // An affine map keeps the ray's parameter: t in local space is t in world space.
    let o = inv.transform_point3(origin) - center;
    let d = inv.transform_vector3(dir);
    let (mut t0, mut t1) = (f32::NEG_INFINITY, f32::INFINITY);
    for i in 0..3 {
        if d[i].abs() < 1e-12 {
            if o[i].abs() > half[i] {
                return None;
            }
            continue;
        }
        let a = (-half[i] - o[i]) / d[i];
        let b = (half[i] - o[i]) / d[i];
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    (t0 <= t1 && t0 >= 0.0).then_some((t0, t1))
}

/// A sphere around the placed box (world centre, radius), for a cheap range test first.
pub fn bounding_sphere(to_world: &Affine3A, center: Vec3, half: Vec3) -> (Vec3, f32) {
    let m = to_world.matrix3;
    let scale = m.x_axis.length().max(m.y_axis.length()).max(m.z_axis.length());
    (to_world.transform_point3(center), half.length() * scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::{Quat, Vec3};

    const ANY: fn(u32) -> bool = |_| true;

    #[test]
    fn collision_hit_on_an_activatable_reference_wins() {
        let p = choose(Some(RayHit { form_id: Some(7), distance: 90.0 }), [], REACH, ANY).unwrap();
        assert_eq!(p, Pick { form_id: 7, distance: 90.0, source: Source::Collision });
    }

    #[test]
    fn beyond_reach_or_not_activatable_is_no_target() {
        assert!(choose(Some(RayHit { form_id: Some(7), distance: 151.0 }), [], REACH, ANY).is_none());
        assert!(choose(Some(RayHit { form_id: Some(7), distance: 50.0 }), [], REACH, |id| id != 7).is_none());
        assert!(choose(Some(RayHit { form_id: None, distance: 50.0 }), [], REACH, ANY).is_none());
        assert!(choose(None, [(3, 151.0)], REACH, ANY).is_none());
    }

    #[test]
    fn bounds_find_items_without_collision() {
        // Nothing collides (the item has no collision; nothing behind within reach).
        let p = choose(None, [(3, 120.0), (4, 80.0), (5, 100.0)], REACH, ANY).unwrap();
        assert_eq!(p, Pick { form_id: 4, distance: 80.0, source: Source::Bounds });
        // An item on a table: the ray enters its box before it hits the table's top.
        let table = RayHit { form_id: Some(100), distance: 95.0 };
        let p = choose(Some(table), [(4, 90.0)], REACH, |id| id != 100).unwrap();
        assert_eq!((p.form_id, p.source), (4, Source::Bounds));
    }

    #[test]
    fn walls_hide_what_is_behind_them() {
        let wall = RayHit { form_id: None, distance: 60.0 };
        assert!(choose(Some(wall), [(4, 80.0)], REACH, ANY).is_none());
        let wall = RayHit { form_id: Some(100), distance: 60.0 };
        assert!(choose(Some(wall), [(4, 80.0)], REACH, |id| id != 100).is_none());
    }

    #[test]
    fn nearest_wins_and_ties_go_to_collision() {
        // An item in front of a door: the item is nearer.
        let door = RayHit { form_id: Some(9), distance: 120.0 };
        assert_eq!(choose(Some(door), [(4, 70.0)], REACH, ANY).unwrap().form_id, 4);
        // The door's own render box at the same distance keeps the collision source.
        let p = choose(Some(door), [(9, 120.0), (9, 119.0)], REACH, ANY).unwrap();
        assert_eq!((p.form_id, p.source), (9, Source::Collision));
        assert_eq!(p.distance, 119.0);
        // Rejected candidates are skipped, not chosen.
        assert_eq!(choose(None, [(1, 10.0), (2, 20.0)], REACH, |id| id == 2).unwrap().form_id, 2);
    }

    /// #3125: a body's render box entered first does not hide an item's box entered
    /// after it; without an item, the body is still picked by its box; a precise box
    /// behind a collision hit stays hidden.
    #[test]
    fn an_item_beside_a_body_wins_over_the_body_s_box() {
        // The ray enters the ghoul's box (8, rough) at 60 and the ammo box's (4) at 95.
        let p = choose_precise_first(None, [(8, 60.0, true), (4, 95.0, false)], REACH, ANY).unwrap();
        assert_eq!(p, Pick { form_id: 4, distance: 95.0, source: Source::Bounds });
        // Plain `choose` (the console's click) keeps the nearest.
        assert_eq!(choose(None, [(8, 60.0), (4, 95.0)], REACH, ANY).unwrap().form_id, 8);
        // Only the body: searched by its box.
        let p = choose_precise_first(None, [(8, 60.0, true)], REACH, ANY).unwrap();
        assert_eq!((p.form_id, p.source), (8, Source::Bounds));
        // An item that cannot be taken does not stand in the body's way.
        assert_eq!(choose_precise_first(None, [(8, 60.0, true), (4, 95.0, false)], REACH, |id| id != 4).unwrap().form_id, 8);
        // The floor at 90 hides an item box entered at 95.
        let floor = RayHit { form_id: None, distance: 90.0 };
        assert_eq!(choose_precise_first(Some(floor), [(8, 60.0, true), (4, 95.0, false)], REACH, ANY).unwrap().form_id, 8);
        // A living actor's collision hit nearer than its box: collision, as before.
        let actor = RayHit { form_id: Some(8), distance: 70.0 };
        let p = choose_precise_first(Some(actor), [(8, 65.0, true)], REACH, ANY).unwrap();
        assert_eq!((p.form_id, p.source), (8, Source::Collision));
    }

    /// #3231: Elder Lyons asleep in his bed. The ray enters the bed's box (20) before
    /// his rough box (8), and the bed's precise box wins; the bed then yields to him
    /// while his body is within reach.
    #[test]
    fn a_bed_in_use_yields_to_its_sleeper_in_reach() {
        let (bed, lyons, crate_) = (20, 8, 4);
        let at = |d: f32| move |id: u32| (id == bed).then_some((lyons, d));
        let picked = choose_precise_first(None, [(bed, 67.0, false), (lyons, 80.0, true)], REACH, ANY);
        assert_eq!(picked.unwrap().form_id, bed);
        assert_eq!(to_occupant(picked, at(72.0), REACH, ANY), Some(Pick { form_id: lyons, distance: 72.0, source: Source::Occupant }));
        // By the bed's collision as well.
        let hit = Some(RayHit { form_id: Some(bed), distance: 140.0 });
        assert_eq!(to_occupant(choose_precise_first(hit, [], REACH, ANY), at(149.0), REACH, ANY).unwrap().form_id, lyons);
        // Aimed at the foot of a wide bed with him beyond reach: the bed stays.
        assert_eq!(to_occupant(picked, at(REACH + 1.0), REACH, ANY), picked);
        // A free bed stays the bed.
        let free = choose_precise_first(None, [(bed, 67.0, false)], REACH, ANY);
        assert_eq!(to_occupant(free, |_| None, REACH, ANY), free);
        // A sleeper that cannot be activated now (interaction disabled) leaves the bed.
        assert_eq!(to_occupant(picked, at(72.0), REACH, |id| id != lyons), picked);
        // Something else picked, or nothing: unchanged.
        let item = choose(None, [(crate_, 50.0)], REACH, ANY);
        assert_eq!(to_occupant(item, at(72.0), REACH, ANY), item);
        assert_eq!(to_occupant(None, at(72.0), REACH, ANY), None);
    }

    /// #3323: a suitcase lying on a bed at the Bison Steve. Along the ray the bed's box
    /// (20) runs from 60 to 140, the suitcase's (5) from 98 to 102, the mattress's
    /// collision is hit at 110.
    #[test]
    fn a_suitcase_lying_in_a_bed_s_box_wins_over_the_bed() {
        const BED: u32 = 20;
        const SUITCASE: u32 = 5;
        let lyons = 8;
        let (bed_box, case_box) = ((BED, 60.0, 140.0, false), (SUITCASE, 98.0, 102.0, false));
        let at = |id: u32, distance: f32| Some(RayHit { form_id: Some(id), distance });
        let furniture = |id: u32| id == BED;
        let pick = |hit: Option<RayHit>, boxes: &[(u32, f32, f32, bool)], accept: fn(u32) -> bool| {
            choose_precise_first(hit, set_aside_enclosing(hit, boxes, REACH, furniture, accept), REACH, accept)
        };
        // Before: the bed's box, entered first, won.
        let all = [bed_box, case_box].map(|b| (b.0, b.1, b.3));
        assert_eq!(choose_precise_first(at(BED, 110.0), all, REACH, ANY).unwrap().form_id, BED);
        // Now the suitcase, whether it has collision of its own or not.
        assert_eq!(pick(at(BED, 110.0), &[bed_box, case_box], ANY), Some(Pick { form_id: SUITCASE, distance: 98.0, source: Source::Bounds }));
        assert_eq!(pick(at(SUITCASE, 99.0), &[bed_box, case_box], ANY).unwrap().form_id, SUITCASE);
        assert_eq!(pick(None, &[case_box, bed_box], ANY).unwrap().form_id, SUITCASE);
        // The bare mattress beside it: the bed, by its collision or its box.
        assert_eq!(pick(at(BED, 110.0), &[bed_box], ANY), Some(Pick { form_id: BED, distance: 60.0, source: Source::Collision }));
        assert_eq!(pick(None, &[bed_box], ANY).unwrap().form_id, BED);
        // A wall at 80 hides the suitcase, which then sets nothing aside.
        let wall = Some(RayHit { form_id: None, distance: 80.0 });
        assert_eq!(pick(wall, &[bed_box, case_box], ANY).unwrap().form_id, BED);
        assert!(pick(wall, &[bed_box, case_box], |id| id != BED).is_none());
        // A suitcase that cannot be taken leaves the bed.
        assert_eq!(pick(at(BED, 110.0), &[bed_box, case_box], |id| id != SUITCASE).unwrap().form_id, BED);
        // Over the same stretch, or only overlapping, neither is set aside: the nearest.
        assert_eq!(pick(None, &[(BED, 98.0, 102.0, false), case_box], ANY).unwrap().form_id, BED);
        assert_eq!(pick(None, &[bed_box, (SUITCASE, 98.0, 150.0, false)], ANY).unwrap().form_id, BED);
        // A sleeper's rough box inside the bed's sets nothing aside: the bed is picked and
        // yields to the sleeper (#3231).
        let picked = pick(None, &[bed_box, (lyons, 80.0, 130.0, true)], ANY);
        assert_eq!(picked.unwrap().form_id, BED);
        let p = to_occupant(picked, |id| (id == BED).then_some((lyons, 90.0)), REACH, ANY).unwrap();
        assert_eq!((p.form_id, p.source), (lyons, Source::Occupant));
        // Only furniture yields: a switch panel's box (30, an activator) from 50 to 130
        // keeps priority over a smaller activator's (31) behind it.
        assert_eq!(pick(None, &[(30, 50.0, 130.0, false), (31, 80.0, 90.0, false)], ANY).unwrap().form_id, 30);
    }

    /// #3323 review: the cutoff for setting a box aside is the raw first hit (a wall at
    /// 80), not the door promoted from 94 behind it (`door_behind`): a suitcase box behind
    /// the wall (85 to 90) inside a door's box (furniture here, 65 to 120) leaves the door.
    #[test]
    fn nothing_behind_the_wall_sets_a_box_aside() {
        const DOOR: u32 = 2;
        let (raw, door) = (Some(RayHit { form_id: None, distance: 80.0 }), Some(RayHit { form_id: Some(DOOR), distance: 94.0 }));
        let boxes = [(DOOR, 65.0, 120.0, false), (5, 85.0, 90.0, false)];
        let kept = set_aside_enclosing(raw, &boxes, REACH, |_| true, ANY);
        assert_eq!(choose_precise_first(door, kept, REACH, ANY), Some(Pick { form_id: DOOR, distance: 65.0, source: Source::Collision }));
        // With the promoted hit as the cutoff the suitcase would win.
        let kept = set_aside_enclosing(door, &boxes, REACH, |_| true, ANY);
        assert_eq!(choose_precise_first(door, kept, REACH, ANY).unwrap().form_id, 5);
    }

    #[test]
    fn distances_to_a_bone_and_to_a_placed_box() {
        let (a, b) = (Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0));
        assert!((segment_distance(Vec3::new(5.0, 3.0, 0.0), a, b) - 3.0).abs() < 1e-5);
        assert!((segment_distance(Vec3::new(-4.0, 3.0, 0.0), a, b) - 5.0).abs() < 1e-5);
        assert!((segment_distance(Vec3::new(1.0, 0.0, 0.0), a, a) - 1.0).abs() < 1e-5);
        // A 10-unit cube scaled ×2 and turned 90° about the up axis, 100 units ahead.
        let t = Affine3A::from_scale_rotation_translation(Vec3::splat(2.0), Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), Vec3::new(0.0, 0.0, -100.0));
        let d = box_distance(Vec3::ZERO, &t, Vec3::ZERO, Vec3::splat(5.0)).unwrap();
        assert!((d - 90.0).abs() < 1e-3, "{d}");
        assert_eq!(box_distance(Vec3::new(0.0, 0.0, -100.0), &t, Vec3::ZERO, Vec3::splat(5.0)), Some(0.0));
        assert!(box_distance(Vec3::ZERO, &Affine3A::from_scale(Vec3::ZERO), Vec3::ZERO, Vec3::ONE).is_none());
    }

    #[test]
    fn ray_enters_a_placed_box() {
        // A 10-unit cube scaled ×2 and turned 45° about the up axis, 100 units ahead.
        let t = Affine3A::from_scale_rotation_translation(Vec3::splat(2.0), Quat::from_rotation_y(std::f32::consts::FRAC_PI_4), Vec3::new(0.0, 0.0, -100.0));
        let half = Vec3::splat(5.0);
        let d = ray_box(Vec3::ZERO, Vec3::NEG_Z, &t, Vec3::ZERO, half).unwrap();
        let (_, out) = ray_box_span(Vec3::ZERO, Vec3::NEG_Z, &t, Vec3::ZERO, half).unwrap();
        assert!((out - (100.0 + 10.0 * 2f32.sqrt())).abs() < 1e-3, "{out}");
        // Corner first: 10 · √2 from the centre after scaling.
        assert!((d - (100.0 - 10.0 * 2f32.sqrt())).abs() < 1e-3, "{d}");
        // Off to the side, behind, and from inside.
        assert!(ray_box(Vec3::new(30.0, 0.0, 0.0), Vec3::NEG_Z, &t, Vec3::ZERO, half).is_none());
        assert!(ray_box(Vec3::ZERO, Vec3::Z, &t, Vec3::ZERO, half).is_none());
        assert!(ray_box(Vec3::new(0.0, 0.0, -100.0), Vec3::NEG_Z, &t, Vec3::ZERO, half).is_none());
        // An off-centre box in the entity's space.
        let d = ray_box(Vec3::new(0.0, 3.0, 0.0), Vec3::NEG_Z, &Affine3A::IDENTITY, Vec3::new(0.0, 3.0, -50.0), Vec3::ONE).unwrap();
        assert!((d - 49.0).abs() < 1e-4);
        // A flat box (a sheet of paper) is still hit face-on.
        let d = ray_box(Vec3::ZERO, Vec3::NEG_Z, &Affine3A::IDENTITY, Vec3::new(0.0, 0.0, -20.0), Vec3::new(1.0, 1.0, 0.0)).unwrap();
        assert!((d - 20.0).abs() < 1e-4);
        // Degenerate transforms are ignored.
        assert!(ray_box(Vec3::ZERO, Vec3::NEG_Z, &Affine3A::from_scale(Vec3::ZERO), Vec3::ZERO, half).is_none());
    }

    #[test]
    fn sphere_covers_the_box() {
        let t = Affine3A::from_scale_rotation_translation(Vec3::new(1.0, 3.0, 1.0), Quat::IDENTITY, Vec3::X * 10.0);
        let (c, r) = bounding_sphere(&t, Vec3::Y, Vec3::ONE);
        assert_eq!(c, Vec3::new(10.0, 3.0, 0.0));
        assert!((r - 3.0 * 3f32.sqrt()).abs() < 1e-5);
    }

    #[test]
    fn a_door_just_behind_a_wall_face_is_the_hit() {
        let (wall, door, crate_) = (1, 2, 3);
        let at = |id: u32, distance: f32| Some(RayHit { form_id: Some(id), distance });
        let usable = |id: u32| id != wall;
        // The Nipton town hall: its mesh at 89.2, the door 8.5 behind it.
        assert_eq!(door_behind(at(wall, 89.2), at(door, 97.7), usable), at(door, 97.7));
        // Too far behind, or the first hit is usable itself: the first hit stands.
        assert_eq!(door_behind(at(wall, 50.0), at(door, 50.0 + DOOR_SLACK + 1.0), usable), at(wall, 50.0));
        assert_eq!(door_behind(at(crate_, 40.0), at(door, 45.0), usable), at(crate_, 40.0));
        // Ground (no reference) counts as not usable; nothing behind leaves it.
        assert_eq!(door_behind(Some(RayHit { form_id: None, distance: 30.0 }), at(door, 35.0), usable), at(door, 35.0));
        assert_eq!(door_behind(at(wall, 89.2), None, usable), at(wall, 89.2));
    }
}
