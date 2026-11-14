//! [design, #2557] Loaded living actors also use this field at Chameleon >= 1,
//! following their ActorRig visual roots (actors.rs); the player path stays separate.
//!
//! The Stealth Boy's look on the player (docs/spec-notes/aid-items.md §9,
//! docs/spec-notes/refraction.md §5): while the player's Chameleon actor value is 1 or
//! more (`fnv_game::aid`, the Stealth Boy's "Stealth Field" effect), the player's body
//! and arms rig are drawn refracting: each mesh shows what lies behind it, bent by its own
//! normals and normal map, with the normal map scrolling so the bend shimmers
//! (`nvf_assets::nif_material`, "Refraction" and "Fire refraction").
//!
//! [hypothesis: the Stealth Field's MGEF names no effect shader (archetype and actor value
//! Chameleon only) [observed], so the look is the engine's; strength [`STRENGTH`] and
//! period [`PERIOD`] are chosen near the game's own refracting shapes (0.04 for Mister
//! Handy's jet, 0.056 for the Incinerator's burst; docs/spec-notes/refraction.md).] Every
//! mesh gets a refracting copy of its material; the copies are dropped and the materials
//! put back when the effect ends. Detection does not read this: it reads the Chameleon
//! value itself (`fnv_game::detection`).
//!
//! The Pip-Boy's screen and its glare are the one exception: while the menus are on it
//! (`pipboy_arm.rs`) they are drawn plain, so the menus can be read and used; the rest of
//! the arm and the Pip-Boy stay see-through, and both refract again once the Pip-Boy is
//! down (owner, 2026-10-23) [design].
//!
//! Log lines: `stealth field: on (Chameleon 75): <n> meshes see-through (refracting,
//! strength 0.030)` and `stealth field: off`.

use crate::limbs::ActorRig;
use crate::pipboy_arm::PipBoyPart;
use crate::player_body::PlayerBody;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use fnv_sim::FormId;
use nvf_assets::nif_material::NifMaterial;
use nvf_play::pipboy::PipBoyScreen;
use nvf_play::script::ScriptState;
use nvf_play::RefVisual;
use std::collections::{HashMap, HashSet};

/// The Chameleon actor value [documented: xEdit ActorValue, `schemas/fnv/common.json`].
const CHAMELEON: u16 = 49;

/// The screen shift of the field: screen UV per unit of view-space normal [hypothesis,
/// docs/spec-notes/refraction.md §5]; the material's strength is this over
/// `nif_material::REFRACTION_SCREEN`.
pub const STRENGTH: f32 = 0.03;

/// Seconds for the normal maps to scroll once, the shimmer [hypothesis].
pub const PERIOD: f32 = 2.0;

/// The cut of a blended part (hair, eyelashes) drawn refracting [hypothesis].
const BLEND_CUT: f32 = 0.5;

/// The meshes made see-through and their own materials.
#[derive(Resource, Default)]
pub struct StealthField {
    pub on: bool,
    originals: HashMap<Entity, Handle<NifMaterial>>,
}

pub struct StealthFieldPlugin;

impl Plugin for StealthFieldPlugin {
    fn build(&self, app: &mut App) {
        // After the Pip-Boy has put the menus on its screen or taken them off, so the
        // screen is skipped exactly while the menus are on it.
        app.init_resource::<StealthField>()
            .init_resource::<ActorFields>()
            .add_systems(Update, apply.after(nvf_play::player::BodySet).after(crate::pipboy_arm::show_screen))
            .add_systems(PostUpdate, apply_actors);
    }
}

impl StealthField {
    /// Give `e` its own material back and stop making it see-through: the material it
    /// had before the field, or `None` when the field had not touched it. The
    /// see-through copy is dropped.
    pub fn take_original(&mut self, e: Entity, current: &Handle<NifMaterial>, materials: &mut Assets<NifMaterial>) -> Option<Handle<NifMaterial>> {
        let original = self.originals.remove(&e)?;
        materials.remove(current);
        Some(original)
    }

    /// As the field does when it makes `e` see-through, for a test of what takes it back.
    #[cfg(test)]
    pub(crate) fn keep_original(&mut self, e: Entity, original: Handle<NifMaterial>) {
        self.originals.insert(e, original);
    }
}

/// The see-through copy of a material: refracting, keeping the outline of its cut.
pub fn see_through(m: &NifMaterial) -> NifMaterial {
    let mut m = m.clone();
    let cut = match m.base.alpha_mode {
        AlphaMode::Mask(cut) => cut,
        AlphaMode::Opaque => m.extension.alpha_cut,
        _ => m.extension.alpha_cut.max(BLEND_CUT),
    };
    nvf_assets::nif_spawn::refraction_material(&mut m, STRENGTH / nvf_assets::nif_material::REFRACTION_SCREEN, PERIOD);
    m.extension.alpha_cut = cut;
    m
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply(
    script: Option<Res<ScriptState>>,
    body: Res<PlayerBody>,
    pip_screen: Option<Res<PipBoyScreen>>,
    parts: Query<&PipBoyPart>,
    mut field: ResMut<StealthField>,
    children: Query<&Children>,
    mut meshes: Query<&mut MeshMaterial3d<NifMaterial>>,
    mut materials: ResMut<Assets<NifMaterial>>,
) {
    let chameleon = script.as_ref().and_then(|s| s.rt.as_ref()).map_or(0.0, |rt| fnv_game::functions::actor_value_temporary(&rt.game, FormId::PLAYER_REF, CHAMELEON));
    let want = chameleon >= 1.0;
    if want {
        // Every frame: a weapon drawn or an outfit changed brings new meshes.
        let mut fresh = 0;
        let menus_on_screen = pip_screen.is_some_and(|s| s.on_model);
        for root in body.roots() {
            for e in children.iter_descendants(root) {
                if field.originals.contains_key(&e) || (menus_on_screen && matches!(parts.get(e), Ok(PipBoyPart::Screen | PipBoyPart::Glare))) {
                    continue;
                }
                let Ok(mut handle) = meshes.get_mut(e) else { continue };
                let Some(copy) = materials.get(&handle.0).map(see_through) else { continue };
                let copy = materials.add(copy);
                field.originals.insert(e, std::mem::replace(&mut handle.0, copy));
                fresh += 1;
            }
        }
        if !field.on {
            info!("stealth field: on (Chameleon {chameleon:.0}): {fresh} meshes see-through (refracting, strength {STRENGTH:.3})");
            field.on = true;
        }
    } else if field.on {
        for (e, original) in field.originals.drain() {
            if let Ok(mut handle) = meshes.get_mut(e) {
                materials.remove(&handle.0);
                handle.0 = original;
            }
        }
        field.on = false;
        info!("stealth field: off");
    }
    // Meshes despawned meanwhile.
    field.originals.retain(|e, _| meshes.contains(*e));
}

/// [design, #2557] Separate from the player's field and Pip-Boy material ownership.
#[derive(Resource, Default)]
pub(crate) struct ActorFields(HashMap<Entity, ActorField>);

impl ActorFields {
    /// [design, #2557; actor_materials::Posing] Animated shapes can write our private
    /// copy in place, rather than retaining a refracting clone after the field ends.
    pub(crate) fn owns(&self, entity: Entity, material: &Handle<NifMaterial>) -> bool {
        self.0.values().any(|field| field.originals.get(&entity).is_some_and(|(_, copy)| copy == material))
    }
}

struct ActorField {
    reference: u32,
    name: String,
    originals: HashMap<Entity, (Handle<NifMaterial>, Handle<NifMaterial>)>,
    pending: bool,
}

/// [design, #2557] Inputs from the loaded visual root and the script runtime.
#[derive(Clone)]
struct ActorTarget {
    root: Entity,
    reference: u32,
    name: String,
    chameleon: f64,
    dead: bool,
}

#[derive(SystemParam)]
struct ActorDrawing<'w, 's> {
    fields: ResMut<'w, ActorFields>,
    children: Query<'w, 's, &'static Children>,
    parents: Query<'w, 's, &'static ChildOf>,
    #[allow(clippy::type_complexity)]
    meshes: ParamSet<'w, 's, (
        Query<'w, 's, Entity, Or<(Changed<Children>, Changed<ChildOf>, Changed<MeshMaterial3d<NifMaterial>>)>>,
        Query<'w, 's, &'static mut MeshMaterial3d<NifMaterial>>,
    )>,
    materials: ResMut<'w, Assets<NifMaterial>>,
}

impl ActorDrawing<'_, '_> {
    /// [design, #2557] Never remove a material supplied by another rendering system.
    fn restore(&mut self, e: Entity, original: Handle<NifMaterial>, copy: Handle<NifMaterial>) {
        let mut meshes = self.meshes.p1();
        if let Ok(mut current) = meshes.get_mut(e) {
            if current.0 == copy {
                current.0 = original;
            }
        }
        self.materials.remove(&copy);
    }

    fn sync(&mut self, targets: impl IntoIterator<Item = ActorTarget>) {
        // [design, #2557] Hierarchy edits and new/replaced material handles dirty only
        // their ancestors. Idle actors never walk descendants or access materials.
        let mut dirty = HashSet::new();
        for e in &self.meshes.p0() {
            dirty.insert(e);
            dirty.extend(self.parents.iter_ancestors(e));
        }
        let targets: Vec<_> = targets.into_iter().collect();
        let loaded: HashSet<_> = targets.iter().map(|t| t.root).collect();
        // [design, #2557] Restore detached parts before another actor can adopt them.
        let roots: Vec<_> = self.fields.0.keys().copied().collect();
        for root in roots {
            if !loaded.contains(&root) {
                let field = self.fields.0.remove(&root).unwrap();
                for (e, (original, copy)) in field.originals {
                    self.restore(e, original, copy);
                }
                info!("stealth field: {} 0x{:08X} off", field.name, field.reference);
            } else if dirty.contains(&root) {
                let descendants: HashSet<_> = std::iter::once(root).chain(self.children.iter_descendants(root)).collect();
                let mut field = self.fields.0.remove(&root).unwrap();
                let detached: Vec<_> = field.originals.keys().copied().filter(|e| !descendants.contains(e)).collect();
                for e in detached {
                    let (original, copy) = field.originals.remove(&e).unwrap();
                    self.restore(e, original, copy);
                }
                self.fields.0.insert(root, field);
            }
        }
        for target in targets {
            let ActorTarget { root, reference, name, chameleon, dead } = target;
            let want = !dead && chameleon >= 1.0;
            if !want {
                if let Some(field) = self.fields.0.remove(&root) {
                    for (e, (original, copy)) in field.originals {
                        self.restore(e, original, copy);
                    }
                    info!("stealth field: {} 0x{:08X} off", field.name, field.reference);
                }
                continue;
            }
            let fresh = !self.fields.0.contains_key(&root);
            if !fresh && !dirty.contains(&root) && !self.fields.0[&root].pending {
                continue;
            }
            let mut field = self.fields.0.remove(&root).unwrap_or_else(|| ActorField { reference, name, originals: HashMap::new(), pending: false });
            let descendants: HashSet<_> = std::iter::once(root).chain(self.children.iter_descendants(root)).collect();
            field.pending = false;
            for e in descendants {
                let mut meshes = self.meshes.p1();
                let Ok(mut current) = meshes.get_mut(e) else {
                    if let Some((_, copy)) = field.originals.remove(&e) {
                        self.materials.remove(&copy);
                    }
                    continue;
                };
                if field.originals.get(&e).is_some_and(|(_, copy)| current.0 == *copy) {
                    continue;
                }
                // [design, #2557] A replacement becomes the new original, so outfit or
                // texture changes survive uncloaking; dispose only of our old copy.
                if let Some((_, copy)) = field.originals.remove(&e) {
                    self.materials.remove(&copy);
                }
                let Some(copy) = self.materials.get(&current.0).map(see_through) else {
                    field.pending = true;
                    continue;
                };
                let copy = self.materials.add(copy);
                let original = std::mem::replace(&mut current.0, copy.clone());
                field.originals.insert(e, (original, copy));
            }
            if fresh {
                info!("stealth field: {} 0x{reference:08X} on (Chameleon {chameleon:.0})", field.name);
            }
            self.fields.0.insert(root, field);
        }
    }
}

/// [design, #2557; actors.rs] ActorRig + RefVisual identifies loaded actor visual roots,
/// including bind-pose actors. PostUpdate sees scripts and newly attached outfits/weapons.
fn apply_actors(script: Option<Res<ScriptState>>, actors: Query<(Entity, &ActorRig), With<RefVisual>>, mut drawing: ActorDrawing) {
    let game = script.as_ref().and_then(|s| s.rt.as_ref()).map(|rt| &rt.game);
    drawing.sync(actors.iter().filter(|(_, rig)| rig.reference != FormId::PLAYER_REF.0).map(|(root, rig)| {
        let reference = FormId(rig.reference);
        ActorTarget {
            root,
            reference: rig.reference,
            name: game.map_or_else(|| format!("0x{:08X}", rig.base), |g| g.data.name(FormId(rig.base))),
            chameleon: game.map_or(0.0, |g| fnv_game::functions::actor_value_temporary(g, reference, CHAMELEON)),
            dead: game.is_some_and(|g| g.is_dead(reference)),
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct Targets(Vec<ActorTarget>);

    #[derive(Resource, Default)]
    struct Writes(usize);

    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<ActorFields>()
            .init_resource::<Assets<NifMaterial>>()
            .init_resource::<Targets>()
            .init_resource::<Writes>()
            .add_systems(PostUpdate, (
                |targets: Res<Targets>, mut drawing: ActorDrawing| drawing.sync(targets.0.iter().cloned()),
                |changed: Query<Entity, Changed<MeshMaterial3d<NifMaterial>>>, mut writes: ResMut<Writes>| writes.0 = changed.iter().count(),
            ).chain());
        app
    }

    fn actor(app: &mut App, chameleon: f64) -> Entity {
        let root = app.world_mut().spawn_empty().id();
        let reference = 0xFF000000 + app.world().resource::<Targets>().0.len() as u32;
        app.world_mut().resource_mut::<Targets>().0.push(ActorTarget { root, reference, name: "TestActor".into(), chameleon, dead: false });
        root
    }

    fn mesh(app: &mut App, parent: Entity) -> (Entity, Handle<NifMaterial>) {
        let material = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(nvf_assets::nif_material::nif_material(StandardMaterial::default()));
        let e = app.world_mut().spawn((ChildOf(parent), MeshMaterial3d(material.clone()))).id();
        (e, material)
    }

    fn worn(app: &App, e: Entity) -> Handle<NifMaterial> {
        app.world().get::<MeshMaterial3d<NifMaterial>>(e).unwrap().0.clone()
    }

    fn refracts(app: &App, e: Entity) -> bool {
        app.world().resource::<Assets<NifMaterial>>().get(&worn(app, e)).unwrap().extension.refract
    }

    #[test]
    fn actors_cloak_independently_at_one_and_restore_on_drop_death_and_recloak() {
        let mut app = app();
        let a = actor(&mut app, 95.0);
        let b = actor(&mut app, 0.5);
        let (body, original) = mesh(&mut app, a);
        let (plain, plain_original) = mesh(&mut app, b);
        app.update();
        assert!(refracts(&app, body));
        assert_eq!(worn(&app, plain), plain_original);
        let copy = worn(&app, body);
        {
            let mut targets = app.world_mut().resource_mut::<Targets>();
            targets.0[0].chameleon = 0.0;
            targets.0[1].chameleon = 1.0;
        }
        app.update();
        assert_eq!(worn(&app, body), original);
        assert!(!app.world().resource::<Assets<NifMaterial>>().contains(copy.id()));
        assert!(refracts(&app, plain));
        app.world_mut().resource_mut::<Targets>().0[0].chameleon = 95.0;
        app.update();
        assert!(refracts(&app, body));
        app.world_mut().resource_mut::<Targets>().0[0].dead = true;
        app.update();
        assert_eq!(worn(&app, body), original);
        assert!(refracts(&app, plain));
        app.world_mut().resource_mut::<Targets>().0[0].dead = false;
        app.update();
        assert!(refracts(&app, body));
    }

    #[test]
    fn unchanged_actors_do_not_write_or_allocate_materials() {
        let mut app = app();
        let a = actor(&mut app, 95.0);
        let (body, _) = mesh(&mut app, a);
        app.update();
        let copy = worn(&app, body);
        let count = app.world().resource::<Assets<NifMaterial>>().len();
        for _ in 0..3 {
            app.update();
            assert_eq!(app.world().resource::<Writes>().0, 0);
            assert_eq!(app.world().resource::<Assets<NifMaterial>>().len(), count);
            assert_eq!(worn(&app, body), copy);
        }
    }

    #[test]
    fn nested_outfit_and_weapon_arrivals_and_replacements_follow_the_field() {
        let mut app = app();
        let a = actor(&mut app, 95.0);
        let (body, _) = mesh(&mut app, a);
        app.update();
        let outfit = app.world_mut().spawn(ChildOf(a)).id();
        let (part, part_original) = mesh(&mut app, outfit);
        let (weapon, weapon_original) = mesh(&mut app, outfit);
        app.update();
        assert!(refracts(&app, part));
        assert!(refracts(&app, weapon));
        let old_copy = worn(&app, body);
        let replacement = app.world_mut().resource_mut::<Assets<NifMaterial>>().add(nvf_assets::nif_material::nif_material(StandardMaterial { base_color: Color::BLACK, ..default() }));
        app.world_mut().entity_mut(body).insert(MeshMaterial3d(replacement.clone()));
        app.update();
        assert!(refracts(&app, body));
        assert!(!app.world().resource::<Assets<NifMaterial>>().contains(old_copy.id()));
        app.world_mut().resource_mut::<Targets>().0[0].chameleon = 0.0;
        app.update();
        assert_eq!(worn(&app, body), replacement);
        assert_eq!(worn(&app, part), part_original);
        assert_eq!(worn(&app, weapon), weapon_original);
    }

    #[test]
    fn material_animation_writes_the_owned_copy_without_retaining_refraction() {
        use fnv_nif::anim::{Channel, Sequence, ValueTrack};
        use nvf_assets::anim_play::{BlendPlayer, BoundClip, Clip, Layer, MaterialTarget, MaterialTrack};
        use nvf_assets::material_anim::{Look, StoredLook};
        use std::sync::Arc;

        let mut app = app();
        let a = actor(&mut app, 95.0);
        let (body, original) = mesh(&mut app, a);
        app.world_mut().entity_mut(body).insert((nvf_assets::actor::ShapeName("testshape".into()), StoredLook(Look::stored([1.0; 3], 1.0, 1.0))));
        app.update();
        let copy = worn(&app, body);
        let mut clip = Clip::from_sequence(&Sequence {
            block: 0, name: Some("synthetic glow".into()), start_time: 0.0, stop_time: 1.0,
            frequency: 1.0, cycle_type: 0, accum_root_name: None, text_keys: Vec::new(), tracks: Vec::new(),
        });
        clip.materials.push(MaterialTrack {
            shape: "testshape".into(), target: MaterialTarget::EmissiveMult,
            values: Arc::new(ValueTrack::Float(Channel::Static(Some([2.0])))), cycle: None, priority: 0,
        });
        let mut player = BlendPlayer::new(Vec::new(), Arc::new(Vec::new()));
        player.layers[0] = Some(Layer { clip: Arc::new(BoundClip::bind(Arc::new(clip), |_| None)), phase: 0.0, weight: 1.0 });
        app.world_mut().entity_mut(a).insert(player);
        app.add_systems(Update, crate::actor_materials::pose);
        app.update();
        assert_eq!(worn(&app, body), copy);
        assert_eq!(app.world().resource::<Assets<NifMaterial>>().get(&copy).unwrap().base.emissive.red, 2.0);
        app.world_mut().resource_mut::<Targets>().0[0].chameleon = 0.0;
        app.update();
        assert_eq!(worn(&app, body), original);
        assert!(!refracts(&app, body));
    }

    #[test]
    fn detached_and_transferred_parts_restore_before_the_new_owner_cloaks() {
        let mut app = app();
        let a = actor(&mut app, 95.0);
        let b = actor(&mut app, 95.0);
        let holder = app.world_mut().spawn(ChildOf(a)).id();
        let (part, original) = mesh(&mut app, holder);
        app.update();
        let copy = worn(&app, part);
        app.world_mut().entity_mut(holder).insert(ChildOf(b));
        app.update();
        assert!(refracts(&app, part));
        assert!(!app.world().resource::<Assets<NifMaterial>>().contains(copy.id()));
        app.world_mut().entity_mut(holder).remove::<ChildOf>();
        app.update();
        assert_eq!(worn(&app, part), original);
        assert!(app.world().resource::<ActorFields>().0.values().all(|f| f.originals.is_empty()));
    }

    #[test]
    fn unloaded_actor_restores_surviving_meshes_and_releases_despawned_copies() {
        let mut app = app();
        let a = actor(&mut app, 95.0);
        let (body, original) = mesh(&mut app, a);
        let (weapon, _) = mesh(&mut app, a);
        app.update();
        let weapon_copy = worn(&app, weapon);
        app.world_mut().despawn(weapon);
        app.update();
        assert!(!app.world().resource::<Assets<NifMaterial>>().contains(weapon_copy.id()));
        let body_copy = worn(&app, body);
        app.world_mut().resource_mut::<Targets>().0.clear();
        app.update();
        assert_eq!(worn(&app, body), original);
        assert!(!app.world().resource::<Assets<NifMaterial>>().contains(body_copy.id()));
        assert!(app.world().resource::<ActorFields>().0.is_empty());
    }

    #[test]
    fn the_copy_refracts_and_keeps_its_cut() {
        let m = nvf_assets::nif_material::nif_material(StandardMaterial { base_color: Color::WHITE, alpha_mode: AlphaMode::Mask(0.4), ..default() });
        let c = see_through(&m);
        assert!(c.extension.refract);
        assert!(matches!(c.base.alpha_mode, AlphaMode::Opaque));
        assert!((c.extension.refraction * nvf_assets::nif_material::REFRACTION_SCREEN - STRENGTH).abs() < 1e-6);
        assert!((c.extension.fire_period - PERIOD).abs() < 1e-6);
        assert!((c.extension.alpha_cut - 0.4).abs() < 1e-6);
        let hair = nvf_assets::nif_material::nif_material(StandardMaterial { alpha_mode: AlphaMode::Blend, ..default() });
        assert!((see_through(&hair).extension.alpha_cut - BLEND_CUT).abs() < 1e-6);
    }
}
