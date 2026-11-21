//! #3323: something lying on a bed (a suitcase at the Bison Steve) is targeted, not the
//! bed whose render box encloses it (`pick::set_aside_enclosing`).
use super::*;

/// A render entity of `form_id`: a box of half size `half` centred at game point `at`.
fn visual_sized(app: &mut App, form_id: u32, at: [f32; 3], half: f32) {
    let aabb = Aabb { center: Vec3::ZERO.into(), half_extents: Vec3::splat(half).into() };
    let gt = GlobalTransform::from_translation(space::to_bevy(at));
    app.world_mut().spawn((RefVisual(form_id), aabb, gt, InheritedVisibility::VISIBLE, Visibility::Inherited));
}

/// The player at the origin looking east, eye at 100; the bed's collision (the mattress)
/// hit at 110, its render box from 60 to 140 along the ray; the chest (the suitcase) a
/// box from 98 to 102 inside it when `item`; a wall hit at 40 when `wall`.
fn bed_scene(item: bool, wall: bool) -> App {
    let mut colliding = vec![(BED_REF, 120.0)];
    if wall {
        // An unnamed activator: collision the player cannot activate.
        colliding.push((TRIGGER_REF, 50.0));
    }
    let mut app = scene(&colliding);
    visual_sized(&mut app, BED_REF, [100.0, 0.0, 100.0], 40.0);
    if item {
        visual_sized(&mut app, CHEST_REF, [100.0, 0.0, 100.0], 2.0);
    }
    app
}

#[test]
fn a_suitcase_lying_on_a_bed_is_the_target_and_opens() {
    let mut app = bed_scene(true, false);
    app.update();
    assert_eq!(target(&app), Some((CHEST_REF, Source::Bounds)));
    assert_eq!(app.world().resource::<Target>().0.as_ref().unwrap().action, Action::Open);
    press_e(&mut app);
    assert_eq!(drain::<OpenContainer>(&mut app).iter().map(|m| m.container).collect::<Vec<_>>(), vec![CHEST_REF]);
}

#[test]
fn the_bare_mattress_is_the_bed() {
    let mut app = bed_scene(false, false);
    app.update();
    assert_eq!(target(&app), Some((BED_REF, Source::Collision)));
}

#[test]
fn a_wall_before_the_bed_hides_both() {
    let mut app = bed_scene(true, true);
    app.update();
    assert_eq!(target(&app), None);
}
