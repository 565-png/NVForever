//! #3233: the Vault 3 captives behind the cell bars (JailCellDoorway01, one collision
//! object on layer 3 Transparent) are talked to through them; what lies behind bars
//! otherwise stays out of reach.
use super::*;
use fnv_phys::HitVolume;

/// Bars: a thin slab 30 units ahead of the eye across the view, on `layer`, owned by a
/// static the simulation does not offer (no placed reference in it, as the bars' STAT).
fn bars(app: &mut App, layer: u8) {
    let slab = CollisionObject { layer, shape: Shape::Box { half_extents: [2.0, 40.0, 80.0] }.into(), ..boxed(10.0) };
    app.world_mut().resource_mut::<PhysState>().world.add_ref(&RefPose { form_id: 0x7780, position: [30.0, 0.0, 100.0], rotation: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0], scale: 1.0 }, &[slab]);
}

/// A living actor's hit capsule standing at `x` on the view line (front face at x - 20).
fn actor_at(app: &mut App, x: f32) {
    let v = HitVolume { feet: [x, 0.0, 0.0], radius: 20.0, height: 130.0, dead: false };
    app.world_mut().resource_mut::<PhysState>().world.set_actor_volume(NPC_REF, v);
}

fn targeted(app: &App) -> Option<(u32, Source, Action)> {
    app.world().resource::<Target>().0.as_ref().map(|t| (t.form_id, t.source, t.action.clone()))
}

#[test]
fn an_actor_behind_transparent_bars_is_talked_to() {
    for layer in [layers::TRANSPARENT, layers::TRANSPARENT_SMALL, layers::TRANSPARENT_SMALL_ANIM] {
        let mut app = scene(&[]);
        bars(&mut app, layer);
        actor_at(&mut app, 120.0);
        app.update();
        assert_eq!(targeted(&app), Some((NPC_REF, Source::Collision, Action::Talk)), "layer {layer}");
        let d = app.world().resource::<Target>().0.as_ref().unwrap().distance;
        assert!((d - 100.0).abs() < 1.0, "the actor's capsule, not the bars: {d}");
    }
}

#[test]
fn opaque_bars_reach_and_a_wall_between_still_hide_the_actor() {
    // Bars on a solid layer hide the actor as before.
    let mut app = scene(&[]);
    bars(&mut app, layers::STATIC);
    actor_at(&mut app, 120.0);
    app.update();
    assert_eq!(targeted(&app), None);
    // Beyond reach behind the bars: nothing.
    let mut app = scene(&[]);
    bars(&mut app, layers::TRANSPARENT);
    actor_at(&mut app, 190.0);
    app.update();
    assert_eq!(targeted(&app), None);
    // A wall (any reference's solid box) between the bars and the actor hides it.
    let mut app = scene(&[(0x999, 70.0)]);
    bars(&mut app, layers::TRANSPARENT);
    actor_at(&mut app, 120.0);
    app.update();
    assert_eq!(targeted(&app), None);
}

#[test]
fn items_and_containers_behind_transparent_bars_stay_out_of_reach() {
    // A chest by its collision behind the bars: not opened through them.
    let mut app = scene(&[(CHEST_REF, 100.0)]);
    bars(&mut app, layers::TRANSPARENT);
    app.update();
    assert_eq!(targeted(&app), None);
    // A stimpak by its render box behind the bars: not taken through them.
    let mut app = scene(&[]);
    bars(&mut app, layers::TRANSPARENT);
    visual(&mut app, STIM_REF, [60.0, 0.0, 100.0]);
    app.update();
    assert_eq!(targeted(&app), None);
    // In front of the bars it still is.
    let mut app = scene(&[]);
    bars(&mut app, layers::TRANSPARENT);
    visual(&mut app, STIM_REF, [20.0, 0.0, 100.0]);
    actor_at(&mut app, 120.0);
    app.update();
    assert_eq!(targeted(&app).map(|t| (t.0, t.1)), Some((STIM_REF, Source::Bounds)));
}

#[test]
fn e_on_the_actor_behind_the_bars_talks() {
    let mut app = scene(&[]);
    bars(&mut app, layers::TRANSPARENT);
    actor_at(&mut app, 120.0);
    app.update();
    press_e(&mut app);
    // Without the scripts Talk is a line saying the actor has nothing to say
    // (`outcome::effects`): the activation went to the actor, not the bars.
    assert!(app.world().resource::<HudMessage>().text().contains("Rick Lancer"), "{}", app.world().resource::<HudMessage>().text());
}
