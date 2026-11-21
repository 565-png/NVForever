//! Talking through bars and fences (#3233): when the crosshair ray stops on collision on
//! a see-through layer and nothing else is offered, a living actor the player may talk to
//! standing just behind it, within reach, is the target.
//!
//! The rule [design, docs/spec-notes/simulation.md §5 "Talking through bars"]: actors
//! see through these layers [documented: GECK wiki "Collision layer": TRANSPARENT,
//! TRANSPARENT SMALL, "Actors can see through this layer type"], and so may talk through
//! them; the original's crosshair on them is not known (observer-checks.md #249). Only
//! Talk passes: items, containers, doors, bodies and pockets behind bars or a window stay
//! out of reach, as render-bounds picks still count only before the first collision hit
//! (`pick::choose`).

use super::pick::{Pick, RayHit, Source};
use fnv_phys::{layers, Filter, LayerSet, Owner, PhysWorld};

/// The layers the crosshair may talk through: Transparent (3), Transparent Small (26),
/// Transparent Small Anim (28), the ones sight passes (nvforever `detection.rs`
/// `see_through`, less the Invisible Wall, which the activation ray already passes).
pub fn talk_layers() -> LayerSet {
    LayerSet::from_layers(&[layers::TRANSPARENT, layers::TRANSPARENT_SMALL, layers::TRANSPARENT_SMALL_ANIM])
}

/// Whether a collision hit is a reference's collider on one of [`talk_layers`].
pub fn is_see_through(owner: Owner) -> bool {
    matches!(owner, Owner::Ref { layer, .. } if talk_layers().contains(layer))
}

/// The actor the view ray meets behind see-through collision: when `first` (the
/// activation ray's first hit, cast with `filter`) is on one of [`talk_layers`], the ray
/// is cast again from `eye` passing them too; the actor whose hit volume or character it
/// meets first within `reach`, if that is what it meets first. A wall, door or anything
/// else between the bars and the actor hides the actor.
pub fn actor_behind(world: &mut PhysWorld, eye: [f32; 3], dir: [f32; 3], reach: f32, first: &fnv_phys::RayHit, filter: &Filter) -> Option<RayHit> {
    if !is_see_through(first.owner) {
        return None;
    }
    let mut past = filter.clone();
    past.exclude_layers = past.exclude_layers.union(talk_layers());
    let h = world.queries().ray(eye, dir, reach, &past)?;
    let form_id = match h.owner {
        Owner::Actor { form_id, .. } => form_id,
        // An NPC's character is added under its reference's id (`PhysState::add_npc`);
        // the player's (0) is never an actor behind bars.
        Owner::Character { id } if id != 0 => id,
        _ => return None,
    };
    Some(RayHit { form_id: Some(form_id), distance: h.distance })
}

/// The crosshair's pick with [`actor_behind`]'s actor: `picked` when there is one (an
/// item or anything before the bars wins); else the actor behind, within `reach`, when
/// the player may talk to it now (`talks`: its prompt is Talk).
pub fn talk_through(picked: Option<Pick>, behind: Option<RayHit>, reach: f32, talks: impl FnOnce(u32) -> bool) -> Option<Pick> {
    if picked.is_some() {
        return picked;
    }
    let b = behind?;
    let id = b.form_id?;
    (b.distance <= reach && talks(id)).then_some(Pick { form_id: id, distance: b.distance, source: Source::Collision })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_see_through_layers_pass() {
        let on = |layer| Owner::Ref { form_id: 1, object: 0, layer };
        assert!(is_see_through(on(layers::TRANSPARENT)));
        assert!(is_see_through(on(layers::TRANSPARENT_SMALL)));
        assert!(is_see_through(on(layers::TRANSPARENT_SMALL_ANIM)));
        assert!(!is_see_through(on(layers::STATIC)));
        assert!(!is_see_through(on(layers::ANIM_STATIC)));
        assert!(!is_see_through(Owner::Terrain { x: 0, y: 0 }));
        assert!(!is_see_through(Owner::Actor { form_id: 1, layer: layers::BIPED, part: 0 }));
    }

    #[test]
    fn an_actor_behind_is_picked_only_when_nothing_else_is_and_it_talks() {
        let behind = Some(RayHit { form_id: Some(9), distance: 120.0 });
        let p = talk_through(None, behind, 150.0, |_| true).unwrap();
        assert_eq!(p, Pick { form_id: 9, distance: 120.0, source: Source::Collision });
        // Not one to talk to (a body, a pocket, a hostile actor): nothing.
        assert!(talk_through(None, behind, 150.0, |_| false).is_none());
        // Out of reach: nothing.
        assert!(talk_through(None, Some(RayHit { form_id: Some(9), distance: 151.0 }), 150.0, |_| true).is_none());
        // Something offered before the bars wins.
        let item = Pick { form_id: 4, distance: 20.0, source: Source::Bounds };
        assert_eq!(talk_through(Some(item), behind, 150.0, |_| true), Some(item));
    }
}
