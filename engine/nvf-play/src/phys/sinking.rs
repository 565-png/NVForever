//! Diagnostics for bodies that sink through the terrain (#2753): dropped weapons and corpses
//! that settled on the ground in exteriors and then went through it [observed: player
//! reports, build 0.7.0, issue #2753]. The cause is not known; these lines say, at the moment
//! a body goes under, where it is against the terrain under its own position, what the
//! physics world has there (the terrain collider, other collision) and what it touches.
//! They decide nothing in the game. Lines: `dropped 0x… woke after resting`, `dropped 0x…
//! sank:` (`PhysState::settle_created`) and `ragdoll: actor … sank:` (nvforever
//! `ragdoll.rs`); docs/spec-notes/physics.md §4.3.

use super::{trace::owner_name, PhysState};
use fnv_phys::{Filter, Key};

/// A body's centre (a dropped item's centre of mass, a ragdoll bone's position) more than
/// this far under the terrain height at its own x, y has sunk into the ground, game units
/// [design, #2753: over five times the terrain's 3.5-unit contact shell, and more than the
/// half thickness of a weapon or a bone lying on the surface].
pub const SANK_NOTE: f32 = 20.0;

/// How far above the higher of the body and the terrain the ground ray starts, and how far
/// down it looks from there, game units [design, #2753].
pub const GROUND_RAY_UP: f32 = 200.0;
pub const GROUND_RAY_DOWN: f32 = 800.0;

/// The most contacts or ray hits a line lists [design, #2753].
const LISTED: usize = 6;

/// Of `points` (an index and a game-space position), the one deepest under the terrain
/// height `terrain` gives at its own x, y, when that depth is more than [`SANK_NOTE`]: its
/// index, its depth and the terrain height there. Points with no terrain under them are
/// left out.
pub fn deepest_under(points: impl IntoIterator<Item = (usize, [f32; 3])>, terrain: impl Fn(f32, f32) -> Option<f32>) -> Option<(usize, f32, f32)> {
    points
        .into_iter()
        .filter_map(|(i, p)| terrain(p[0], p[1]).map(|t| (i, t - p[2], t)))
        .filter(|&(_, depth, _)| depth > SANK_NOTE)
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

impl PhysState {
    /// Every hit of a ray straight down from `at`, `down` long (nearest first, at most a
    /// few), as `<owner> at z <z>`; "nothing" without one. Actors (characters, hit volumes,
    /// ragdolls) and the layers the character passes through are left out, and `exclude`.
    pub fn ground_hits(&mut self, at: [f32; 3], down: f32, exclude: Option<Key>) -> String {
        let filter = Filter { exclude: exclude.into_iter().collect(), no_actors: true, exclude_layers: fnv_phys::layers::character_ignores(), ..Default::default() };
        let hits = self.ray_all(at, [0.0, 0.0, -1.0], down, filter);
        if hits.is_empty() {
            return "nothing".into();
        }
        hits.iter().take(LISTED).map(|h| format!("{} at z {:.1}", owner_name(Some(h.owner)), h.point[2])).collect::<Vec<_>>().join(", ")
    }

    /// What a reference's colliders touch, as of the last narrow-phase update: each other
    /// owner once, with its deepest contact's distance (negative: overlapping), deepest
    /// first; "nothing" without contacts.
    pub fn contacts_text(&self, form_id: u32) -> String {
        let mut contacts = self.world.ref_contacts(form_id);
        contacts.sort_by(|a, b| a.2.total_cmp(&b.2));
        let mut seen = Vec::new();
        let mut out = Vec::new();
        for (_, owner, dist) in contacts {
            let key = owner.map(|o| o.key());
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            out.push(format!("{} {dist:.1}", owner_name(owner)));
            if out.len() == LISTED {
                break;
            }
        }
        if out.is_empty() {
            "nothing".into()
        } else {
            out.join(", ")
        }
    }

    /// The created dynamic references found moving this frame (`settle_created`): those
    /// that `woke` after resting (seconds at rest, where), and those `awake` not yet logged
    /// as sunk, checked against the terrain under their centre of mass.
    pub(super) fn note_created_sinking(&mut self, woke: Vec<(u32, f32, [f32; 3])>, awake: Vec<(u32, [f32; 3])>) {
        for (id, rested, pos) in woke {
            let touching = self.contacts_text(id);
            bevy::log::info!("dropped 0x{id:08X} woke after resting {rested:.2} s at ({:.1}, {:.1}, {:.2}): touching {touching}", pos[0], pos[1], pos[2]);
        }
        let now = self.world.stepped_secs();
        for (id, linvel) in awake {
            let Some(c) = self.world.ref_center(id) else { continue };
            let Some((_, depth, t)) = deepest_under([(0, c)], |x, y| self.terrain_z(x, y)) else { continue };
            let Some(cb) = self.created.get_mut(&id) else { continue };
            cb.sank = true;
            let since = (now - cb.added_at) as f32;
            let rest = cb.rested_at.map_or_else(|| "never at rest".to_string(), |r| format!("at rest from {:.2} s", (r - cb.added_at) as f32));
            let low = self.world.ref_lowest_z(id).map_or_else(|| "-".to_string(), |z| format!("{z:.1}"));
            let ground = self.ground_hits([c[0], c[1], t.max(c[2]) + GROUND_RAY_UP], GROUND_RAY_DOWN, Some(Key::Ref(id)));
            let touching = self.contacts_text(id);
            let speed = linvel.iter().map(|v| v * v).sum::<f32>().sqrt();
            bevy::log::info!(
                "dropped 0x{id:08X} sank: centre at ({:.1}, {:.1}, {:.1}), {depth:.1} under the terrain there (z {t:.1}), lowest point {low}, speed {speed:.0} units/s (vertical {:.0}), {since:.2} s after its collision was added, {rest}; ground there: {ground}; touching {touching}",
                c[0],
                c[1],
                c[2],
                linvel[2]
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plane rising 0.5 units a unit eastwards from z 100 at x 0.
    fn slope(x: f32, _y: f32) -> Option<f32> {
        Some(100.0 + 0.5 * x)
    }

    #[test]
    fn a_point_on_or_just_under_the_surface_has_not_sunk() {
        assert_eq!(deepest_under([(0, [0.0, 0.0, 100.0]), (1, [10.0, 0.0, 105.0 - SANK_NOTE + 0.5])], slope), None);
    }

    #[test]
    fn the_terrain_under_each_point_is_its_own() {
        // 200 units uphill the terrain is at 200: a point at 150 there is 50 under it,
        // though it is above the terrain under the first point (the corpse guard's reading).
        let found = deepest_under([(0, [0.0, 0.0, 101.0]), (1, [200.0, 0.0, 150.0]), (2, [100.0, 0.0, 120.0])], slope);
        let (i, depth, t) = found.expect("the uphill point has sunk");
        assert_eq!(i, 1);
        assert!((depth - 50.0).abs() < 1e-3 && (t - 200.0).abs() < 1e-3);
        // Downhill of the first point, lower than it but on the surface: not sunk.
        assert_eq!(deepest_under([(0, [400.0, 0.0, 301.0]), (1, [0.0, 0.0, 99.0])], slope), None);
    }

    #[test]
    fn points_with_no_terrain_under_them_are_left_out() {
        assert_eq!(deepest_under([(0, [0.0, 0.0, -1000.0])], |_, _| None), None);
    }
}
