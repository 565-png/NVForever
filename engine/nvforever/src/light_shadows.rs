//! Shadows of placed lights, the settings row Lamp shadows (`light-shadows`;
//! docs/design/graphics-enhanced.md §3.15). An addition of NVForever's own, not a fact
//! about the original game: there placed lights cast none, and no LIGH record of the
//! masters sets the Spot Shadow flag [observed: docs/handoff/esm-data-audit-2026-10-25.md,
//! item 6]. Off by default.
//!
//! - **Which lights**: up to the row's budget (Off, 2, 4 or 8) of the placed LIGH lights
//!   ([`SessionLight`] with a point or spot light) that are drawn, whose lit sphere meets
//!   the main camera's view and reaches within [`REACH`] of the camera. The most important
//!   come first: brightness × (range / distance)², the distance taken as at least half
//!   the range [engineering choice: the share of the view a light brightens, roughly]. A
//!   light that already casts keeps its place until another scores [`KEEP`] times as
//!   much, so shadows do not flip on and off as the camera moves.
//! - **What casts**: the sun's layers (`sky::SHADOW_LAYER`, `sky::BODY_LAYER` and the
//!   world's layer 0), so the player's body casts a lamp's shadow as it casts the sun's;
//!   the first-person arms do not. A light lights the views its layers meet, so the main
//!   and the reflection camera as before (sky.rs, the sun).
//! - **Near plane** [`NEAR`]: what lies nearer the light than that casts nothing from it,
//!   so the bulb, shade or flames a light sits in do not black out the room [engineering
//!   choice; `NVF_LIGHT_SHADOW_NEAR` overrides it for a run].
//! - **Maps**: a point light draws a cube of six faces (Bevy's `PointLightShadowMap`,
//!   1024² each); a spot light one layer of the sun's shadow texture (the Shadow
//!   resolution row's side). Each shadowed light is a view of its own for the renderer
//!   (a point light six), which costs render-thread time as well as GPU time
//!   (graphics-enhanced.md §5.5).
//! - The diffuse light of the display-value sum takes the shadow too
//!   (`lights::display_shader`), as the flashlight's always has. The flashlight is not
//!   counted against the budget: it casts whenever it is on (flashlight.rs).

use bevy::camera::primitives::{Frustum, Sphere};
use bevy::camera::visibility::{RenderLayers, VisibilitySystems};
use bevy::light::SimulationLightSystems;
use bevy::prelude::*;
use bevy::transform::TransformSystems;

use crate::lights::{MovingLight, SessionLight};

/// The row's choices, in its order: the name shown and saved, and how many lights cast.
pub const BUDGETS: [(&str, usize); 4] = [("Off", 0), ("2", 2), ("4", 4), ("8", 8)];

/// How far beyond its own range a light may lie from the camera and still cast, game
/// units (about 29 m) [engineering choice: farther lamps' shadows are a few pixels].
pub const REACH: f32 = 2048.0;

/// How much more another light must score to take a casting light's place.
pub const KEEP: f32 = 1.5;

/// Shadow near plane of a placed light, game units (about 23 cm) [engineering choice:
/// clears the bulbs, shades and fire barrels' rims lights sit in; the flashlight's is 8].
pub const NEAR: f32 = 16.0;

/// How many placed lights cast shadows (0: none). Set by the row and `--light-shadows`.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightShadows {
    pub budget: usize,
}

/// On a placed light while it casts shadows.
#[derive(Component)]
pub struct CastingLight;

/// The budget the command line asks for, as one of [`BUDGETS`] (the nearest at or under it).
pub fn budget_of(lights: usize) -> usize {
    BUDGETS.iter().rev().map(|b| b.1).find(|&b| b <= lights).unwrap_or(0)
}

pub struct LightShadowsPlugin(pub usize);

impl Plugin for LightShadowsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(LightShadows { budget: budget_of(self.0) }).add_systems(
            PostUpdate,
            choose
                .after(TransformSystems::Propagate)
                .after(VisibilitySystems::UpdateFrusta)
                .before(SimulationLightSystems::AssignLightsToClusters)
                .before(SimulationLightSystems::UpdateLightFrusta),
        );
    }
}

/// A light's claim to a shadow: its brightness (`intensity` × its colour's largest
/// channel) × (range / distance)², the distance at least half the range; `None` when it is
/// out of reach of the camera at `distance`.
pub fn score(brightness: f32, range: f32, distance: f32) -> Option<f32> {
    if range <= 0.0 || brightness <= 0.0 || distance - range > REACH {
        return None;
    }
    let d = distance.max(range * 0.5);
    Some(brightness * (range / d) * (range / d))
}

/// The lights that cast: the `budget` best of `(id, score, casting now)`, a casting light's
/// score counted [`KEEP`] times; ties keep the order given.
pub fn pick<T: Copy>(candidates: &mut [(T, f32, bool)], budget: usize) -> Vec<T> {
    candidates.sort_by(|a, b| {
        let (sa, sb) = (a.1 * if a.2 { KEEP } else { 1.0 }, b.1 * if b.2 { KEEP } else { 1.0 });
        sb.total_cmp(&sa)
    });
    candidates.iter().take(budget).map(|c| c.0).collect()
}

/// The near plane in use: [`NEAR`], or `NVF_LIGHT_SHADOW_NEAR` (game units) for a run.
fn near() -> f32 {
    static NEAR_Z: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *NEAR_Z.get_or_init(|| {
        match std::env::var("NVF_LIGHT_SHADOW_NEAR").ok().and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| *v > 0.0) {
            Some(v) => {
                info!("light shadows: NVF_LIGHT_SHADOW_NEAR={v}");
                v
            }
            None => NEAR,
        }
    })
}

/// `NVF_LIGHT_SHADOW_BIAS=<depth>,<normal>` (a test aid): the shadow biases of the casting
/// lights in place of Bevy's defaults, for comparing.
fn biases() -> Option<(f32, f32)> {
    static BIAS: std::sync::OnceLock<Option<(f32, f32)>> = std::sync::OnceLock::new();
    *BIAS.get_or_init(|| {
        let v = std::env::var("NVF_LIGHT_SHADOW_BIAS").ok()?;
        let (d, n) = v.split_once(',')?;
        let b = (d.trim().parse().ok()?, n.trim().parse().ok()?);
        info!("light shadows: NVF_LIGHT_SHADOW_BIAS depth {} normal {}", b.0, b.1);
        Some(b)
    })
}

/// The layers a casting light draws its shadow casters from (the sun's, sky.rs).
fn casting_layers() -> RenderLayers {
    RenderLayers::from_layers(&[0, crate::sky::SHADOW_LAYER, crate::sky::BODY_LAYER])
}

type Placed<'a> = (
    Entity,
    &'a GlobalTransform,
    &'a InheritedVisibility,
    Option<&'a mut PointLight>,
    Option<&'a mut SpotLight>,
    Option<&'a MovingLight>,
    Has<CastingLight>,
);

/// Give the budget's lights their shadows and take them from the rest.
fn choose(
    mut commands: Commands,
    settings: Res<LightShadows>,
    camera: Query<(&GlobalTransform, &Frustum), crate::fly::WorldCamera>,
    mut lights: Query<Placed, With<SessionLight>>,
    mut logged: Local<Option<(usize, usize, usize, usize)>>,
) {
    let chosen: Vec<Entity> = match (settings.budget, camera.iter().next()) {
        (0, _) | (_, None) => Vec::new(),
        (budget, Some((cam, frustum))) => {
            let at = cam.translation();
            let mut candidates: Vec<(Entity, f32, bool)> = lights
                .iter()
                .filter(|(_, _, seen, ..)| seen.get())
                .filter_map(|(e, t, _, point, spot, moving, casting)| {
                    let (range, intensity, colour) = match (&point, &spot) {
                        (Some(p), _) => (p.range, p.intensity, p.color),
                        (None, Some(s)) => (s.range, s.intensity, s.color),
                        _ => return None,
                    };
                    let c = colour.to_linear();
                    let brightness = moving.map_or(intensity, |m| m.full) * c.red.max(c.green).max(c.blue);
                    let centre = t.translation();
                    if !frustum.intersects_sphere(&Sphere { center: centre.into(), radius: range }, true) {
                        return None;
                    }
                    score(brightness, range, centre.distance(at)).map(|s| (e, s, casting))
                })
                .collect();
            pick(&mut candidates, budget)
        }
    };
    let (near, bias) = (near(), biases());
    let (mut points, mut spots, mut drawn) = (0, 0, 0);
    for (e, _, seen, point, spot, _, casting) in &mut lights {
        if seen.get() && (point.is_some() || spot.is_some()) {
            drawn += 1;
        }
        let cast = chosen.contains(&e);
        if let Some(mut p) = point {
            if cast {
                points += 1;
            }
            if p.shadow_maps_enabled != cast {
                p.shadow_maps_enabled = cast;
            }
            if cast && p.shadow_map_near_z != near {
                p.shadow_map_near_z = near;
            }
            if let (true, Some((d, n))) = (cast, bias) {
                if (p.shadow_depth_bias, p.shadow_normal_bias) != (d, n) {
                    (p.shadow_depth_bias, p.shadow_normal_bias) = (d, n);
                }
            }
        }
        if let Some(mut s) = spot {
            if cast {
                spots += 1;
            }
            if s.shadow_maps_enabled != cast {
                s.shadow_maps_enabled = cast;
            }
            if cast && s.shadow_map_near_z != near {
                s.shadow_map_near_z = near;
            }
            if let (true, Some((d, n))) = (cast, bias) {
                if (s.shadow_depth_bias, s.shadow_normal_bias) != (d, n) {
                    (s.shadow_depth_bias, s.shadow_normal_bias) = (d, n);
                }
            }
        }
        match (cast, casting) {
            (true, false) => {
                commands.entity(e).insert((CastingLight, casting_layers()));
            }
            (false, true) => {
                commands.entity(e).remove::<(CastingLight, RenderLayers)>();
            }
            _ => {}
        }
    }
    let now = (settings.budget, points, spots, drawn.min(1));
    if *logged != Some(now) {
        *logged = Some(now);
        if settings.budget == 0 {
            info!("light shadows: off");
        } else {
            info!("light shadows: {} of the budget of {} cast ({points} point, {spots} spot lights; {drawn} placed lights drawn)", points + spots, settings.budget);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_are_a_short_list_and_the_command_line_takes_the_nearest_below() {
        assert!(BUDGETS.len() <= 7, "a list row (settings-page.md §0)");
        assert_eq!(BUDGETS[0], ("Off", 0));
        assert_eq!(budget_of(0), 0);
        assert_eq!(budget_of(1), 0);
        assert_eq!(budget_of(3), 2);
        assert_eq!(budget_of(4), 4);
        assert_eq!(budget_of(100), 8);
    }

    #[test]
    fn nearer_brighter_and_larger_lights_score_higher() {
        let near = score(1.0, 400.0, 300.0).unwrap();
        let far = score(1.0, 400.0, 1200.0).unwrap();
        assert!(near > far);
        assert!(score(2.0, 400.0, 1200.0).unwrap() > far);
        assert!(score(1.0, 800.0, 1200.0).unwrap() > far);
        // Inside half its range a light scores as at half its range.
        assert_eq!(score(1.0, 400.0, 10.0), score(1.0, 400.0, 200.0));
        // Out of reach, dark or without range: none.
        assert_eq!(score(1.0, 400.0, 400.0 + REACH + 1.0), None);
        assert_eq!(score(0.0, 400.0, 100.0), None);
        assert_eq!(score(1.0, 0.0, 100.0), None);
    }

    #[test]
    fn a_casting_light_keeps_its_place_until_another_scores_keep_times_as_much() {
        let mut c = vec![(1, 1.0, true), (2, 1.4, false)];
        assert_eq!(pick(&mut c, 1), vec![1]);
        let mut c = vec![(1, 1.0, true), (2, 1.6, false)];
        assert_eq!(pick(&mut c, 1), vec![2]);
        let mut c = vec![(1, 3.0, false), (2, 1.0, false), (3, 2.0, false)];
        assert_eq!(pick(&mut c, 2), vec![1, 3]);
        assert_eq!(pick(&mut c, 0), Vec::<i32>::new());
        assert_eq!(pick(&mut c, 8).len(), 3);
    }
}
