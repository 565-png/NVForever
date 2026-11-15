//! [design, #2652] Aim points for references absent from the shooter's drawn actors.

use fnv_combat::vec::V3;
use fnv_game::Game;
use fnv_sim::FormId;

/// [design, #2652] Use the current placed position, as the AI's facing does
/// (`fnv_ai::goal::place_of_ref`). Add the base's OBND centre when available; without
/// bounds (a marker), aim at the position itself [hypothesis: observer check 212].
pub(crate) fn target_place(game: &Game, target: FormId) -> Option<(V3, V3)> {
    if !game.present(target) || game.extra.get(&target).is_some_and(|e| e.dead) {
        return None;
    }
    let feet = fnv_ai::goal::place_of_ref(game, target)?.pos;
    let mut aim = feet;
    if let Some(bounds) = fnv_game::functions::base_of(game, target).and_then(|b| game.data.object_bounds.get(&b)) {
        for i in 0..3 {
            aim[i] += (f32::from(bounds[i]) + f32::from(bounds[i + 3])) * 0.5;
        }
    }
    Some((feet, aim))
}
