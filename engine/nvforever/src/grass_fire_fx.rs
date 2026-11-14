//! Grass fires on screen (`nvf_play::grass_fire`, docs/design/grass-fire.md): the burn
//! map copied to the grass materials' texture when it changes (`crate::grass::GrassBurn`),
//! the cells that unloaded forgotten so their grass grows back whole, the weather's wind
//! handed to the fires, and a small fire model on each burning patch.
//!
//! The flames are the base game's ambient small fire, `effects\ambient\fxfiresmall01.nif`
//! (the model placed fires use), at most [`MAX_FLAMES`] at once, each turned and sized at
//! random and shrinking away over [`SHRINK_SECS`] when its patch burns out [design].
//!
//! Log: `grass fire fx: …` once when the flame model is loaded (or not), and at exit
//! `grass fire fx summary: …`.

use crate::grass::GrassBurn;
use crate::world::{Streamer, WorldRoot};
use bevy::prelude::*;
use nvf_assets::model::ModelCtx;
use nvf_play::grass_fire::{centre, FireEvent, GrassFires, Key};
use std::collections::HashMap;

/// The flame model on a burning patch.
pub const FLAME_MODEL: &str = r"effects\ambient\fxfiresmall01.nif";
/// The most flame models at once [design: frame cost].
pub const MAX_FLAMES: usize = 150;
/// Seconds a flame takes to shrink away once its patch burnt out [chosen].
pub const SHRINK_SECS: f32 = 0.8;

#[derive(Resource, Default)]
struct FlameFx {
    /// The flame on each burning patch that has one.
    flames: HashMap<Key, Entity>,
    /// The model, once tried (`None` inside: it did not load).
    model: Option<Option<std::sync::Arc<nvf_assets::model::PreparedModel>>>,
    spawned: u32,
    skipped: u32,
    uploads: u32,
    cells_forgotten: u32,
}

/// A flame shrinking away.
#[derive(Component)]
struct Shrink {
    left: f32,
    full: f32,
}

pub struct GrassFireFxPlugin;

impl Plugin for GrassFireFxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FlameFx>()
            .add_systems(Update, (sync, shrink).chain().after(nvf_play::PlaySet::Activation).after(crate::world::stream).run_if(resource_exists::<GrassFires>))
            .add_systems(Last, report);
    }
}

/// See the module notes.
#[allow(clippy::too_many_arguments)]
fn sync(
    mut commands: Commands,
    mut gf: ResMut<GrassFires>,
    mut fx: ResMut<FlameFx>,
    streamer: Res<Streamer>,
    burn: Res<GrassBurn>,
    sky: Option<Res<crate::sky::SkyState>>,
    mut models: ModelCtx,
    root: Query<Entity, With<WorldRoot>>,
) {
    let fires = &mut gf.fires;
    // The weather's wind: game (x, y) is Bevy (x, -z) (`crate::tree::wind_vector`).
    if let Some(sky) = sky.as_deref() {
        let w = crate::tree::wind_vector(sky);
        fires.wind = [w.x * w.w, -w.z * w.w];
    }
    // Cells no longer loaded (nor being spawned): forgotten, so they grow back.
    let gone: Vec<(i32, i32)> = fires.cells().filter(|c| streamer.interior.is_some() || !(streamer.loaded.contains_key(c) || streamer.spawning.contains_key(c))).collect();
    for c in gone {
        fires.forget_cell(c);
        fx.cells_forgotten += 1;
    }
    if fires.dirty {
        if let Some(image) = models.images.get_mut(&burn.0) {
            image.data = Some(fires.map.clone());
            fires.dirty = false;
            fx.uploads += 1;
        }
    }
    let events = std::mem::take(&mut fires.events);
    let Ok(world_root) = root.single() else { return };
    for e in events {
        match e {
            FireEvent::Lit { key, z } => {
                if fx.flames.len() >= MAX_FLAMES {
                    fx.skipped += 1;
                    continue;
                }
                let pm = fx.model.get_or_insert_with(|| {
                    let path = fnv_actor::resolve::mesh_path(FLAME_MODEL);
                    let pm = models.model(&path);
                    match &pm {
                        Some(pm) => info!("grass fire fx: flame model {path}: {} shapes, {} nodes", pm.shapes.len(), pm.nodes.len()),
                        None => warn!("grass fire fx: flame model {path} not loaded; fires show only the burning grass"),
                    }
                    pm
                });
                let Some(pm) = pm.clone() else { continue };
                let c = centre(key);
                // Turned and sized at random, fixed by the patch [chosen].
                let r = hash(key);
                let scale = 0.7 + 0.5 * (r & 0xFF) as f32 / 255.0;
                let yaw = ((r >> 8) & 0xFFFF) as f32 / 65535.0 * std::f32::consts::TAU;
                let jitter = Vec2::new(((r >> 24) & 0xF) as f32 / 15.0 - 0.5, ((r >> 28) & 0xF) as f32 / 15.0 - 0.5) * 24.0;
                let place = Transform::from_translation(Vec3::new(c[0] + jitter.x, c[1] + jitter.y, z)).with_rotation(Quat::from_rotation_z(yaw)).with_scale(Vec3::splat(scale));
                let path = fnv_actor::resolve::mesh_path(FLAME_MODEL);
                let (holder, _) = crate::shot_fx::spawn_playing_itself(&mut commands, world_root, &pm, place, &path);
                commands.entity(holder).insert(crate::particles::ModelEmitter::new(path));
                fx.flames.insert(key, holder);
                fx.spawned += 1;
            }
            FireEvent::Out { key } => {
                if let Some(e) = fx.flames.remove(&key) {
                    commands.entity(e).try_insert(Shrink { left: SHRINK_SECS, full: 0.0 });
                }
            }
            _ => {}
        }
    }
}

/// Flames whose patch burnt out shrink away, then go.
fn shrink(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Shrink, &mut Transform)>) {
    let dt = time.delta_secs();
    for (e, mut s, mut t) in &mut q {
        if s.full == 0.0 {
            s.full = t.scale.x.max(1e-3);
        }
        s.left -= dt;
        if s.left <= 0.0 {
            commands.entity(e).try_despawn();
        } else {
            t.scale = Vec3::splat(s.full * (s.left / SHRINK_SECS));
        }
    }
}

/// A patch's fixed random bits.
fn hash(k: Key) -> u32 {
    let mut h = (k.0 as u32).wrapping_mul(0x9E37_79B1) ^ (k.1 as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0xC2B2_AE3D);
    h ^ (h >> 13)
}

fn report(mut exit: MessageReader<AppExit>, fx: Res<FlameFx>) {
    if exit.read().next().is_some() {
        info!(
            "grass fire fx summary: {} flames drawn ({} patches over the cap of {MAX_FLAMES} had none), {} burn map uploads, {} cells forgotten (their grass grows back)",
            fx.spawned, fx.skipped, fx.uploads, fx.cells_forgotten
        );
    }
}
