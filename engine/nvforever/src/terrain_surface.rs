//! [design] Aid-only, bounded 3x3 unlit colour atlas, using terrain.rs's prepared meshes
//! and materials. Missing/incomplete cells have no height entry and exert no influence.

use crate::terrain::TerrainMaterial;
use crate::world::{bevy_to_game, game_to_bevy, Streamer, WorldData};
use bevy::asset::RenderAssetUsages;
use bevy::camera::{visibility::RenderLayers, Hdr, RenderTarget, ScalingMode};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::image::ImageSampler;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};

pub const RESOLUTION: u32 = 512;
const WIDTH: u32 = 3 * RESOLUTION;
const HEIGHT_WIDTH: usize = 3 * 33;
const BAKE_LAYER: usize = 15;
const INVALID_HEIGHT: f32 = -f32::MAX;

#[derive(Component)]
pub struct SurfaceCell {
    pub world: u32,
    pub grid: (i32, i32),
    pub heights: Vec<f32>,
    pub quadrants: Vec<(Mesh, TerrainMaterial)>,
}

impl SurfaceCell {
    pub fn push(&mut self, mesh: &Mesh, mut material: TerrainMaterial) {
        let mut flat = mesh.clone();
        if let Some(VertexAttributeValues::Float32x3(p)) = flat.attribute_mut(Mesh::ATTRIBUTE_POSITION) {
            for v in p { v[2] = 0.0; }
        }
        material.extension.surface_bake = true;
        material.extension.burn = None;
        material.base.fog_enabled = false;
        self.quadrants.push((flat, material));
    }
}

#[derive(Clone)]
pub struct PublishedSurface {
    pub colour: Handle<Image>,
    pub height: Handle<Image>,
    pub origin: (i32, i32),
    pub cells: usize,
}

struct BakeJob {
    root: Entity,
    publication: PublishedSurface,
}

#[derive(Resource, Default)]
pub struct SurfaceCache {
    pub generation: u64,
    signature: Option<(u32, (i32, i32), Vec<Entity>)>,
    job: Option<BakeJob>,
    ready: Option<u64>,
    pub published: Option<PublishedSurface>,
}

// [design] The row stride of the half-float render target is already 256-byte aligned.
fn bake_complete(data: &[u8], slots: &[(i32, i32)]) -> bool {
    slots.iter().all(|&(x, y)| {
        (0..4).all(|q| {
            let px = x as u32 * RESOLUTION + RESOLUTION / 4 + (q % 2) * RESOLUTION / 2;
            let py = WIDTH - 1 - (y as u32 * RESOLUTION + RESOLUTION / 4 + (q / 2) * RESOLUTION / 2);
            let alpha = ((py * WIDTH + px) * 8 + 6) as usize;
            data.get(alpha..alpha + 2).is_some_and(|a| a == [0x00, 0x3c])
        })
    })
}

#[allow(clippy::too_many_arguments)]
pub fn update_cache(
    mut commands: Commands,
    mut cache: ResMut<SurfaceCache>,
    world: Res<WorldData>,
    streamer: Res<Streamer>,
    camera: Query<&GlobalTransform, crate::fly::WorldCamera>,
    cells: Query<(Entity, &SurfaceCell)>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
) {
    let Ok(camera) = camera.single() else { return };
    let eye = bevy_to_game(camera.translation());
    let centre = fnv_world::cell_of(eye.x, eye.y);
    let origin = (centre.0 - 1, centre.1 - 1);
    let mut resident: Vec<_> = cells.iter().filter(|(_, c)| {
        streamer.interior.is_none() && c.world == world.world
            && (origin.0..origin.0 + 3).contains(&c.grid.0)
            && (origin.1..origin.1 + 3).contains(&c.grid.1)
            && c.heights.len() == 33 * 33
    }).collect();
    resident.sort_by_key(|(_, c)| c.grid);
    let signature = (world.world, origin, resident.iter().map(|(e, _)| *e).collect::<Vec<_>>());
    if cache.signature.as_ref() == Some(&signature) {
        if cache.ready == Some(cache.generation) {
            cache.ready = None;
            if let Some(job) = cache.job.take() {
                commands.entity(job.root).despawn();
                cache.published = Some(job.publication);
            }
        }
        return;
    }
    cache.signature = Some(signature);
    cache.generation += 1;
    cache.ready = None;
    cache.published = None;
    if let Some(old) = cache.job.take() { commands.entity(old.root).despawn(); }
    if resident.is_empty() { return; }

    let mut colour = Image::new_fill(
        Extent3d { width: WIDTH, height: WIDTH, depth_or_array_layers: 1 },
        TextureDimension::D2, &[0; 8], TextureFormat::Rgba16Float,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    colour.texture_descriptor.usage |= TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC;
    colour.sampler = ImageSampler::linear();
    let colour = images.add(colour);
    let mut heights = vec![0; HEIGHT_WIDTH * HEIGHT_WIDTH * 4];
    for h in heights.chunks_exact_mut(4) { h.copy_from_slice(&INVALID_HEIGHT.to_le_bytes()); }
    let mut slots = Vec::new();
    let root = commands.spawn((Name::new("Mesh terrain surface bake"), game_to_bevy(), Visibility::Inherited)).id();
    for (_, cell) in &resident {
        let slot = (cell.grid.0 - origin.0, cell.grid.1 - origin.1);
        slots.push(slot);
        for y in 0..33 {
            for x in 0..33 {
                let at = ((slot.1 as usize * 33 + y) * HEIGHT_WIDTH + slot.0 as usize * 33 + x) * 4;
                heights[at..at + 4].copy_from_slice(&cell.heights[y * 33 + x].to_le_bytes());
            }
        }
        for (mesh, material) in &cell.quadrants {
            commands.spawn((
                Mesh3d(meshes.add(mesh.clone())), MeshMaterial3d(materials.add(material.clone())),
                Transform::from_xyz(slot.0 as f32 * 4096.0, slot.1 as f32 * 4096.0, 0.0),
                RenderLayers::layer(BAKE_LAYER), bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver, ChildOf(root),
            ));
        }
    }
    let height = images.add(Image::new(
        Extent3d { width: HEIGHT_WIDTH as u32, height: HEIGHT_WIDTH as u32, depth_or_array_layers: 1 },
        TextureDimension::D2, heights, TextureFormat::Rgba8Unorm,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    ));
    // [design] This independent view covers flat copies; no gameplay camera, lights or fog.
    commands.spawn((
        Camera3d::default(),
        Camera { order: -10, clear_color: ClearColorConfig::Custom(Color::NONE), ..default() },
        RenderTarget::Image(colour.clone().into()), Hdr, Tonemapping::None, Msaa::Off,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed { width: 12288.0, height: 12288.0 },
            near: 0.0, far: 200.0, ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(6144.0, 6144.0, 100.0).looking_at(Vec3::new(6144.0, 6144.0, 0.0), Vec3::Y),
        RenderLayers::layer(BAKE_LAYER), ChildOf(root),
    ));
    // [design] Check all four quadrants of every slot before publishing. A newly created
    // render target alone does not mean its material pipelines/textures have drawn.
    let generation = cache.generation;
    commands.spawn((Readback::texture(colour.clone()), ChildOf(root))).observe(
        move |ev: On<ReadbackComplete>, mut cache: ResMut<SurfaceCache>| {
            if cache.generation == generation && bake_complete(&ev.data, &slots) {
                cache.ready = Some(generation);
            }
        },
    );
    cache.job = Some(BakeJob {
        root,
        publication: PublishedSurface { colour, height, origin, cells: resident.len() },
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_encoding_survives_unorm_texture_load_without_precision_loss() {
        for h in [-12345.75_f32, -0.0, 0.0, 0.125, 5678.5, INVALID_HEIGHT] {
            let b = h.to_le_bytes().map(|b| ((b as f32 / 255.0) * 255.0).round() as u8);
            assert_eq!(f32::from_le_bytes(b).to_bits(), h.to_bits());
        }
    }

    #[test]
    fn publication_requires_every_quadrant_and_rejects_empty_readback() {
        assert!(!bake_complete(&[], &[(0, 0)]));
        let mut data = vec![0; (WIDTH * WIDTH * 8) as usize];
        for px in data.chunks_exact_mut(8) { px[6..8].copy_from_slice(&[0, 0x3c]); }
        assert!(bake_complete(&data, &[(0, 0), (2, 2)]));
        let py = WIDTH - 1 - RESOLUTION / 4;
        data[((py * WIDTH + RESOLUTION / 4) * 8 + 6) as usize] = 1;
        assert!(!bake_complete(&data, &[(0, 0), (2, 2)]));
    }
}
