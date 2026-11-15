//! [design] Aid-only, bounded 3x3 unlit colour atlas, using terrain.rs's prepared meshes
//! and materials. Heights cover every prepared cell; colour validity is per quadrant.

use crate::terrain::TerrainMaterial;
use crate::world::{bevy_to_game, game_to_bevy, Streamer, WorldData};
use bevy::asset::RenderAssetUsages;
use bevy::camera::{visibility::RenderLayers, Hdr, RenderTarget, ScalingMode};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::image::ImageSampler;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_phase::{DrawFunctions, RenderCommand, RenderCommandResult, RenderCommandState, PhaseItem, TrackedRenderPass};
use bevy::core_pipeline::core_3d::Opaque3d;
use bevy::ecs::{query::ROQueryItem, system::SystemParamItem};
use std::{collections::HashSet, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use fnv_world::land::SIDE;

pub use nvf_assets::nif_surface_blend::{RESOLUTION, GRID, HEIGHT_TILE_WIDTH, HEIGHT_TILE_HEIGHT};
const WIDTH: u32 = 3 * RESOLUTION;
const HEIGHT_WIDTH: usize = 3 * HEIGHT_TILE_WIDTH;
const HEIGHT_HEIGHT: usize = 3 * HEIGHT_TILE_HEIGHT;
pub const BAKE_LAYER: usize = 16;
const INVALID_HEIGHT: f32 = -f32::MAX;

#[derive(Clone, Copy, Debug)]
pub enum CapturePath { Folded, Fallback, HeightOnly(&'static str) }
impl CapturePath {
    fn label(self) -> String {
        match self {
            Self::Folded => "folded".into(), Self::Fallback => "fallback".into(),
            Self::HeightOnly(reason) => format!("height-only:{reason}"),
        }
    }
    fn colour_valid(self) -> bool { !matches!(self, Self::HeightOnly(_)) }
}

#[derive(Component)]
pub struct SurfaceCell {
    pub world: u32,
    pub grid: (i32, i32),
    pub heights: Vec<f32>,
    pub quadrants: Vec<(Mesh, TerrainMaterial)>,
    pub paths: [CapturePath; 4],
}

impl SurfaceCell {
    pub fn push(&mut self, quadrant: u8, mesh: &Mesh, mut material: TerrainMaterial, path: CapturePath) {
        let mut flat = mesh.clone();
        if let Some(VertexAttributeValues::Float32x3(p)) = flat.attribute_mut(Mesh::ATTRIBUTE_POSITION) {
            for v in p { v[2] = 0.0; }
        }
        material.extension.surface_bake = true;
        material.extension.burn = None;
        material.base.fog_enabled = false;
        self.quadrants.push((flat, material));
        self.paths[quadrant as usize] = path;
    }
    fn colour_mask(&self) -> u32 {
        self.paths.iter().enumerate().fold(0, |mask, (q, path)| mask | (u32::from(path.colour_valid()) << q))
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
    report: String,
}

#[derive(Resource, Default)]
pub struct SurfaceCache {
    pub generation: u64,
    signature: Option<(u32, (i32, i32), Vec<Entity>)>,
    job: Option<BakeJob>,
    ready: Option<u64>,
    submitted: Arc<Mutex<Option<u64>>>,
    pub published: Option<PublishedSurface>,
}

pub fn cache_origin(point: Vec2) -> (i32, i32) {
    let centre = fnv_world::cell_of(point.x, point.y);
    (centre.0 - 1, centre.1 - 1)
}
// [design] One production height representation: the coefficients of both terrain
// triangles, encoded losslessly. WGSL only selects the triangle and evaluates its plane.
pub fn height_planes(a: f32, east: f32, north: f32, ne: f32) -> [[f32; 3]; 2] {
    [[a, east - a, ne - east], [a, ne - north, north - a]]
}
pub fn height_index(slot: (i32, i32), square: (usize, usize), triangle: usize, coefficient: usize) -> usize {
    let x = slot.0 as usize * HEIGHT_TILE_WIDTH + square.0 * 6 + triangle * 3 + coefficient;
    let y = slot.1 as usize * HEIGHT_TILE_HEIGHT + square.1;
    (y * HEIGHT_WIDTH + x) * 4
}
pub fn residency_mask(slot: (i32, i32), resident: &HashSet<(i32, i32)>) -> u32 {
    let mut mask = 0;
    for y in -1..=1 {
        for x in -1..=1 {
            if resident.contains(&(slot.0 + x, slot.1 + y)) { mask |= 1 << ((y + 1) * 3 + x + 1); }
        }
    }
    mask
}
fn encode_residency(data: &mut [u8], slot: (i32, i32), resident: &HashSet<(i32, i32)>) {
    let at = ((slot.1 as usize * HEIGHT_TILE_HEIGHT + GRID) * HEIGHT_WIDTH + slot.0 as usize * HEIGHT_TILE_WIDTH) * 4;
    data[at..at + 4].copy_from_slice(&(residency_mask(slot, resident) as f32).to_le_bytes());
}
fn encode_colour_mask(data: &mut [u8], slot: (i32, i32), mask: u32) {
    let at = ((slot.1 as usize * HEIGHT_TILE_HEIGHT + GRID) * HEIGHT_WIDTH + slot.0 as usize * HEIGHT_TILE_WIDTH + 1) * 4;
    data[at..at + 4].copy_from_slice(&(mask as f32).to_le_bytes());
}

#[derive(Debug, PartialEq, Eq)]
pub enum LookupStatus { OutOfRange, InvalidHeight, RejectedFootprint, InvalidColourQuadrant, Success }

fn height_texel(data: &[u8], p: (usize, usize)) -> f32 {
    let at = (p.1 * HEIGHT_WIDTH + p.0) * 4;
    f32::from_le_bytes(data[at..at + 4].try_into().unwrap())
}
fn colour_valid(data: &[u8], slot: (i32, i32), quadrant: u32) -> bool {
    let mask = height_texel(data, (slot.0 as usize * HEIGHT_TILE_WIDTH + 1, slot.1 as usize * HEIGHT_TILE_HEIGHT + GRID)) as u32;
    mask & (1 << quadrant) != 0
}
// [design] CPU publication probe, with the same precedence/footprint as the shader.
// This classifies uploaded bytes; it does not assert that GPU bindings are correct.
pub fn lookup_status(data: &[u8], origin: (i32, i32), point: Vec2) -> LookupStatus {
    let cell = fnv_world::cell_of(point.x, point.y);
    let slot = (cell.0 - origin.0, cell.1 - origin.1);
    if !(0..3).contains(&slot.0) || !(0..3).contains(&slot.1) { return LookupStatus::OutOfRange; }
    let local = point / 4096.0 - Vec2::new(cell.0 as f32, cell.1 as f32);
    let square = (local * GRID as f32).floor().as_uvec2().min(UVec2::splat(GRID as u32 - 1));
    let at = height_index(slot, (square.x as usize, square.y as usize), 0, 0);
    if f32::from_le_bytes(data[at..at + 4].try_into().unwrap()) < -1e30 { return LookupStatus::InvalidHeight; }
    let quadrant = u32::from(local.x >= 0.5) + 2 * u32::from(local.y >= 0.5);
    if !colour_valid(data, slot, quadrant) { return LookupStatus::InvalidColourQuadrant; }
    let atlas = (point / 4096.0 - Vec2::new(origin.0 as f32, origin.1 as f32)) / 3.0;
    let tap = (Vec2::new(atlas.x, 1.0 - atlas.y) * WIDTH as f32 - Vec2::splat(0.5)).floor().as_ivec2();
    let residency = height_texel(data, (slot.0 as usize * HEIGHT_TILE_WIDTH, slot.1 as usize * HEIGHT_TILE_HEIGHT + GRID)) as u32;
    for y in 0..2 { for x in 0..2 {
        let pixel = (tap + IVec2::new(x, y)).clamp(IVec2::ZERO, IVec2::splat(WIDTH as i32 - 1));
        let tap_slot = (pixel.x / RESOLUTION as i32, 2 - pixel.y / RESOLUTION as i32);
        let offset = (tap_slot.0 - slot.0 + 1, tap_slot.1 - slot.1 + 1);
        let quadrant = u32::from(pixel.x % RESOLUTION as i32 >= RESOLUTION as i32 / 2)
            + 2 * u32::from(pixel.y % (RESOLUTION as i32) < RESOLUTION as i32 / 2);
        if residency & (1 << (offset.1 * 3 + offset.0)) == 0 || !colour_valid(data, tap_slot, quadrant) {
            return LookupStatus::RejectedFootprint;
        }
    }}
    LookupStatus::Success
}

fn publication_log(generation: u64, origin: (i32, i32), resident: &[String], excluded: &[String], probe: &str, assets: (AssetId<Image>, AssetId<Image>)) -> String {
    format!("mesh terrain blend: publication generation={generation} origin=({}, {}) resident=[{}] excluded=[{}] {probe} colour={:?} height={:?}", origin.0, origin.1, resident.join(";"), excluded.join(";"), assets.0, assets.1)
}
fn publication_probe(data: &[u8], origin: (i32, i32), point: Vec2) -> String {
    let cell = fnv_world::cell_of(point.x, point.y);
    let slot = (cell.0 - origin.0, cell.1 - origin.1);
    let local = point / fnv_world::land::SPACING - Vec2::new(cell.0 as f32, cell.1 as f32) * GRID as f32;
    let square = local.floor().as_uvec2().min(UVec2::splat(GRID as u32 - 1));
    let f = local - square.as_vec2();
    let triangle = usize::from(f.y > f.x);
    let at = height_index(slot, (square.x as usize, square.y as usize), triangle, 0);
    format!("probe-cell={cell:?} probe-slot={slot:?} probe-game-xy={point:?} probe-square={square:?} probe-triangle={triangle} probe-height-bytes={:?} probe-status={:?}", &data[at..at + 4], lookup_status(data, origin, point))
}
pub fn encode_height_cell(data: &mut [u8], slot: (i32, i32), heights: &[f32]) {
    for y in 0..GRID {
        for x in 0..GRID {
            let i = y * SIDE + x;
            let planes = height_planes(heights[i], heights[i + 1], heights[i + SIDE], heights[i + SIDE + 1]);
            for (triangle, plane) in planes.iter().enumerate() {
                for (coefficient, value) in plane.iter().enumerate() {
                    let at = height_index(slot, (x, y), triangle, coefficient);
                    data[at..at + 4].copy_from_slice(&value.to_le_bytes());
                }
            }
        }
    }
}
fn encode_surface_cell(data: &mut [u8], slot: (i32, i32), cell: &SurfaceCell, resident: &HashSet<(i32, i32)>) {
    encode_height_cell(data, slot, &cell.heights);
    encode_residency(data, slot, resident);
    encode_colour_mask(data, slot, cell.colour_mask());
}
fn empty_height_atlas() -> Vec<u8> {
    let mut heights = vec![0; HEIGHT_WIDTH * HEIGHT_HEIGHT * 4];
    for h in heights.as_chunks_mut::<4>().0 { h.copy_from_slice(&INVALID_HEIGHT.to_le_bytes()); }
    heights
}

// [design] Receipts follow DrawMaterial's successful direct draw, not merely readiness
// or a fence. Every frame starts empty, so incomplete frames cannot accumulate to success.
#[derive(Component, Clone, ExtractComponent)]
#[extract_app(RenderApp)]
struct BakeView {
    generation: u64,
    expected: HashSet<Entity>,
    complete: Arc<AtomicBool>,
    drawn: Arc<Mutex<HashSet<Entity>>>,
    submitted: Arc<Mutex<Option<u64>>>,
}

pub struct SurfaceBakePlugin;
impl Plugin for SurfaceBakePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<BakeView>::default());
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.add_systems(RenderStartup, install_bake_receipt)
                .add_systems(Render, reset_receipts.in_set(RenderSystems::Prepare))
                .add_systems(Render, submit_receipts.in_set(RenderSystems::Cleanup));
        }
    }
}

fn install_bake_receipt(world: &mut World) {
    let command = RenderCommandState::<Opaque3d, (bevy::pbr::DrawMaterial, BakeReceipt)>::new(world);
    let functions = world.resource::<DrawFunctions<Opaque3d>>();
    let mut functions = functions.write();
    let id = functions.id::<bevy::pbr::DrawMaterial>();
    functions.draw_functions[id.0 as usize] = Box::new(command);
}
fn reset_receipts(views: Query<&BakeView>, mut cameras: ResMut<bevy::render::camera::SortedCameras>) {
    // Freeze successful generations before another view pass can clear their target.
    cameras.0.retain(|camera| !views.get(camera.entity).is_ok_and(|view| view.complete.load(Ordering::Acquire)));
    for view in &views { view.drawn.lock().unwrap().clear(); }
}
fn complete_draws(expected: &HashSet<Entity>, drawn: &HashSet<Entity>) -> bool {
    !expected.is_empty() && expected.is_subset(drawn)
}
fn submit_receipts(views: Query<&BakeView>) {
    for view in &views {
        if complete_draws(&view.expected, &view.drawn.lock().unwrap()) && !view.complete.swap(true, Ordering::AcqRel) {
            // RenderSystems::Cleanup follows render_system, including queue submission.
            *view.submitted.lock().unwrap() = Some(view.generation);
        }
    }
}
struct BakeReceipt;
impl RenderCommand<Opaque3d> for BakeReceipt {
    type Param = ();
    type ViewQuery = Option<&'static BakeView>;
    type ItemQuery = ();
    fn render<'w>(item: &Opaque3d, view: ROQueryItem<'w, '_, Self::ViewQuery>, _: Option<()>, _: SystemParamItem<'w, '_, Self::Param>, _: &mut TrackedRenderPass<'w>) -> RenderCommandResult {
        if let Some(view) = view {
            // NoAutomaticBatching + NoIndirectDrawing: one successful indexed draw
            // represents this exact quadrant, with no indirect culling ambiguity.
            view.drawn.lock().unwrap().insert(item.main_entity().id());
        }
        RenderCommandResult::Success
    }
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
    // Consume the previous update's receipt first; a newly received acknowledgement
    // waits one further main frame before materials can sample the submitted target.
    let publish = cache.ready.take();
    let received = cache.submitted.lock().unwrap().take();
    cache.ready = received;
    let Ok(camera) = camera.single() else { return };
    let eye = bevy_to_game(camera.translation());
    let origin = cache_origin(eye.truncate());
    let mut resident: Vec<_> = cells.iter().filter(|(_, c)| {
        streamer.interior.is_none() && c.world == world.world
            && (origin.0..origin.0 + 3).contains(&c.grid.0)
            && (origin.1..origin.1 + 3).contains(&c.grid.1)
            && c.heights.len() == SIDE * SIDE
    }).collect();
    resident.sort_by_key(|(_, c)| c.grid);
    let signature = (world.world, origin, resident.iter().map(|(e, _)| *e).collect::<Vec<_>>());
    if cache.signature.as_ref() == Some(&signature) {
        if publish == Some(cache.generation) {
            cache.ready = None;
            if let Some(job) = cache.job.take() {
                commands.entity(job.root).despawn();
                info!("{}", job.report);
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
    colour.texture_descriptor.usage |= TextureUsages::RENDER_ATTACHMENT;
    colour.sampler = ImageSampler::linear();
    let colour = images.add(colour);
    let mut heights = empty_height_atlas();
    let mut expected = HashSet::new();
    let root = commands.spawn((Name::new("Mesh terrain surface bake"), game_to_bevy(), Visibility::Inherited)).id();
    let slots = resident.iter().map(|(_, c)| (c.grid.0 - origin.0, c.grid.1 - origin.1)).collect();
    for (_, cell) in &resident {
        let slot = (cell.grid.0 - origin.0, cell.grid.1 - origin.1);
        encode_surface_cell(&mut heights, slot, cell, &slots);
        for (mesh, material) in &cell.quadrants {
            let entity = commands.spawn((
                bevy::render::batching::NoAutomaticBatching,
                Mesh3d(meshes.add(mesh.clone())), MeshMaterial3d(materials.add(material.clone())),
                Transform::from_xyz(slot.0 as f32 * 4096.0, slot.1 as f32 * 4096.0, 0.0),
                RenderLayers::layer(BAKE_LAYER), bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver, ChildOf(root),
            )).id();
            expected.insert(entity);
        }
    }
    let resident_report: Vec<_> = resident.iter().map(|(_, cell)| format!("({}, {}):q=[{}] colour-mask={:#x}", cell.grid.0, cell.grid.1, cell.paths.map(CapturePath::label).join(","), cell.colour_mask())).collect();
    let mut excluded = Vec::new();
    for y in origin.1..origin.1 + 3 { for x in origin.0..origin.0 + 3 {
        if !resident.iter().any(|(_, c)| c.grid == (x, y)) {
            let reason = if !streamer.loaded.contains_key(&(x, y)) { "not-loaded" }
                else if cells.iter().any(|(_, c)| c.grid == (x, y) && c.world == world.world) { "invalid-prepared-heights" }
                else { "no-prepared-LAND" };
            excluded.push(format!("({x}, {y}):{reason}"));
        }
    }}
    // Eye XY is inside the centre slot, regardless of eye height. This probes the
    // uploaded coefficients beneath that point, not an unobserved mesh fragment.
    let probe = publication_probe(&heights, origin, eye.truncate());
    let height = images.add(Image::new(
        Extent3d { width: HEIGHT_WIDTH as u32, height: HEIGHT_HEIGHT as u32, depth_or_array_layers: 1 },
        TextureDimension::D2, heights, TextureFormat::Rgba8Unorm,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    ));
    // [design] This independent view covers flat copies; no gameplay camera, lights or fog.
    if expected.is_empty() {
        // All cells are height-only. No colour is sampled, so no bake/receipt is needed.
        cache.ready = Some(cache.generation);
    } else { commands.spawn((
        Camera3d::default(), bevy::render::view::NoIndirectDrawing,
        BakeView { generation: cache.generation, expected, complete: default(), drawn: default(), submitted: cache.submitted.clone() },
        Camera { order: -10, clear_color: ClearColorConfig::Custom(Color::NONE), ..default() },
        RenderTarget::Image(colour.clone().into()), Hdr, Tonemapping::None, Msaa::Off,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed { width: 12288.0, height: 12288.0 },
            near: 0.0, far: 200.0, ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(6144.0, 6144.0, 100.0).looking_at(Vec3::new(6144.0, 6144.0, 0.0), Vec3::Y),
        RenderLayers::layer(BAKE_LAYER), ChildOf(root),
    )); }
    let report = publication_log(cache.generation, origin, &resident_report, &excluded, &probe, (colour.id(), height.id()));
    cache.job = Some(BakeJob {
        root,
        publication: PublishedSurface { colour, height, origin, cells: resident.len() },
        report,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn surface(paths: [CapturePath; 4]) -> SurfaceCell {
        SurfaceCell { world: 0, grid: (-17, 0), heights: vec![42.5; SIDE * SIDE], quadrants: vec![], paths }
    }
    #[test]
    fn unsupported_quadrants_and_height_only_cells_keep_all_prepared_heights() {
        let mut data = empty_height_atlas();
        let slots = HashSet::from([(0, 0), (1, 0)]);
        let partial = surface([CapturePath::Folded, CapturePath::HeightOnly("layered-overflow"), CapturePath::Fallback, CapturePath::Folded]);
        let height_only = surface([CapturePath::HeightOnly("non-bindless-layers"); 4]);
        encode_surface_cell(&mut data, (0, 0), &partial, &slots);
        encode_surface_cell(&mut data, (1, 0), &height_only, &slots);
        assert_eq!(partial.colour_mask(), 0b1101);
        assert_eq!(height_only.colour_mask(), 0);
        for slot in [(0, 0), (1, 0)] { for y in 0..GRID { for x in 0..GRID { for triangle in 0..2 {
            let at = height_index(slot, (x, y), triangle, 0);
            assert_eq!(f32::from_le_bytes(data[at..at + 4].try_into().unwrap()), 42.5);
        }}}}
        assert_eq!(lookup_status(&data, (-18, -1), Vec2::new(-18.0 * 4096.0 + 1024.0, -3072.0)), LookupStatus::Success);
        assert_eq!(lookup_status(&data, (-18, -1), Vec2::new(-18.0 * 4096.0 + 3072.0, -3072.0)), LookupStatus::InvalidColourQuadrant);
        assert_eq!(lookup_status(&data, (-18, -1), Vec2::new(-17.0 * 4096.0 + 1024.0, -3072.0)), LookupStatus::InvalidColourQuadrant);
    }
    #[test]
    fn cpu_publication_probe_classifies_all_five_shader_outcomes() {
        let mut data = empty_height_atlas();
        let cell = surface([CapturePath::Folded; 4]);
        let slots = HashSet::from([(0, 0), (1, 0), (0, 1)]);
        for &slot in &slots { encode_surface_cell(&mut data, slot, &cell, &slots); }
        assert_eq!(lookup_status(&data, (0, 0), Vec2::new(-1.0, 100.0)), LookupStatus::OutOfRange);
        assert_eq!(lookup_status(&data, (0, 0), Vec2::splat(6144.0)), LookupStatus::InvalidHeight);
        assert_eq!(lookup_status(&data, (0, 0), Vec2::splat(4095.9)), LookupStatus::RejectedFootprint, "missing diagonal tap");
        assert_eq!(lookup_status(&data, (0, 0), Vec2::splat(1024.0)), LookupStatus::Success);
        encode_colour_mask(&mut data, (0, 0), 0b1110);
        assert_eq!(lookup_status(&data, (0, 0), Vec2::splat(1024.0)), LookupStatus::InvalidColourQuadrant);
        assert_eq!(lookup_status(&data, (0, 0), Vec2::new(2048.1, 1024.0)), LookupStatus::RejectedFootprint, "one bilinear tap crosses into the invalid neighbour quadrant");
        assert_eq!(lookup_status(&data, (0, 0), Vec2::new(3072.0, 1024.0)), LookupStatus::Success, "supported neighbour quadrant is retained");
    }
    #[test]
    fn publication_line_names_paths_exclusions_probe_and_asset_ids() {
        let id = Handle::<Image>::default().id();
        let resident = vec![format!("(-17, 0):q=[{}] colour-mask=0x1", [CapturePath::Folded, CapturePath::HeightOnly("layered-overflow"), CapturePath::HeightOnly("fallback-layers"), CapturePath::HeightOnly("non-bindless-layers")].map(CapturePath::label).join(","))];
        let line = publication_log(7, (-18, -1), &resident, &["(-18, -1):not-loaded".into()], "probe-slot=(1, 1)", (id, id));
        assert_eq!(line, format!("mesh terrain blend: publication generation=7 origin=(-18, -1) resident=[(-17, 0):q=[folded,height-only:layered-overflow,height-only:fallback-layers,height-only:non-bindless-layers] colour-mask=0x1] excluded=[(-18, -1):not-loaded] probe-slot=(1, 1) colour={id:?} height={id:?}"));
        let mut data = empty_height_atlas();
        let cell = surface([CapturePath::Folded; 4]);
        encode_surface_cell(&mut data, (1, 1), &cell, &HashSet::from([(1, 1)]));
        let probe = publication_probe(&data, (-18, -1), Vec2::new(-17.0 * 4096.0 + 1024.0, 1024.0));
        assert!(probe.contains("probe-cell=(-17, 0) probe-slot=(1, 1)"));
        assert!(probe.contains(&format!("probe-height-bytes={:?} probe-status=Success", 42.5f32.to_le_bytes())));
    }
    #[test]
    fn production_render_layer_reservations_are_distinct() {
        let layers = [
            ("world", 0), ("water", crate::reflection::WATER_LAYER),
            ("shadow", crate::sky::SHADOW_LAYER), ("arms", crate::sky::ARMS_LAYER),
            ("body", crate::sky::BODY_LAYER), ("lock", crate::lockpick_view::LOCK_LAYER),
            ("terminal", crate::terminal_view::MONITOR_LAYER), ("blackjack", crate::blackjack_view::TABLE_LAYER),
            ("roulette", crate::roulette_view::ROULETTE_LAYER), ("poker", crate::poker_view::POKER_LAYER),
            ("decal", crate::decals::DECAL_LAYER), ("scope", crate::scope_view::SCOPE_LAYER),
            ("vigor", nvf_play::chargen::vigor::VIGOR_LAYER), ("creator", crate::creator_view::STUDIO_LAYER),
            ("vats", crate::vats_highlight::MASK_LAYER), ("bake", BAKE_LAYER), ("culled", crate::rooms::CULL_LAYER),
        ];
        for (i, a) in layers.iter().enumerate() {
            for b in &layers[i + 1..] { assert_ne!(a.1, b.1, "{} and {}", a.0, b.0); }
        }
        // RenderLayers::layer literal calls in particles.rs/effect_shaders.rs are test
        // fixtures, not further production reservations. Multi-layer views combine these.
    }
    #[test]
    fn l_shaped_residency_includes_cardinals_but_rejects_the_diagonal_tap() {
        let resident = HashSet::from([(0, 0), (1, 0), (0, 1)]);
        let mask = residency_mask((0, 0), &resident);
        for bit in [4, 5, 7] { assert_ne!(mask & (1 << bit), 0); }
        assert_eq!(mask & (1 << 8), 0, "northeast bilinear tap must be rejected");
        let mut full = resident.clone(); full.insert((1, 1));
        assert_ne!(residency_mask((0, 0), &full) & (1 << 8), 0);
        let negative = resident.iter().map(|&(x, y)| (x - 2, y - 1)).collect();
        assert_eq!(mask, residency_mask((-2, -1), &negative));
    }
    #[test]
    fn cache_origin_uses_negative_floor_and_covers_all_slots() {
        assert_eq!(cache_origin(Vec2::new(-0.01, -0.01)), (-2, -2));
        assert_eq!(cache_origin(Vec2::new(-8192.0, -4096.0)), (-3, -2));
        assert_eq!(cache_origin(Vec2::ZERO), (-1, -1));
        let origin = cache_origin(Vec2::new(-8192.0, -4096.0));
        for y in 0..3 { for x in 0..3 {
            let cell = (origin.0 + x, origin.1 + y);
            let point = Vec2::new(cell.0 as f32 * 4096.0 + 64.0, cell.1 as f32 * 4096.0 + 64.0);
            assert_eq!(fnv_world::cell_of(point.x, point.y), cell);
        }}
    }
    #[test]
    fn all_atlas_slots_and_plane_bytes_round_trip() {
        let h: Vec<_> = (0..SIDE * SIDE).map(|i| (i as f32 * 0.125) - 123.75).collect();
        let mut data = vec![0; HEIGHT_WIDTH * HEIGHT_HEIGHT * 4];
        for y in 0..3 { for x in 0..3 {
            encode_height_cell(&mut data, (x, y), &h);
        }}
        for y in 0..3 { for x in 0..3 { for sy in 0..GRID { for sx in 0..GRID {
            let i = sy * SIDE + sx;
            let planes = height_planes(h[i], h[i + 1], h[i + SIDE], h[i + SIDE + 1]);
            for (triangle, plane) in planes.iter().enumerate() { for (c, value) in plane.iter().enumerate() {
                let at = height_index((x, y), (sx, sy), triangle, c);
                let bytes: [u8; 4] = data[at..at + 4].try_into().unwrap();
                let loaded = bytes.map(|b| ((b as f32 / 255.0) * 255.0).round() as u8);
                assert_eq!(f32::from_le_bytes(loaded).to_bits(), value.to_bits());
            }}
        }}}}
    }
    #[test]
    fn receipt_systems_reset_partial_frames_and_freeze_submitted_view() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        app.init_resource::<bevy::render::camera::SortedCameras>();
        let a = app.world_mut().spawn_empty().id();
        let b = app.world_mut().spawn_empty().id();
        let drawn = Arc::new(Mutex::new(HashSet::new()));
        let submitted = Arc::new(Mutex::new(None));
        let camera = app.world_mut().spawn(BakeView {
            generation: 7, expected: HashSet::from([a, b]), complete: default(),
            drawn: drawn.clone(), submitted: submitted.clone(),
        }).id();
        app.world_mut().resource_mut::<bevy::render::camera::SortedCameras>().0.push(bevy::render::camera::SortedCamera {
            entity: camera, order: 0, target: None, output_mode: default(),
        });
        drawn.lock().unwrap().insert(a);
        app.world_mut().run_system_once(submit_receipts).unwrap();
        assert_eq!(*submitted.lock().unwrap(), None);
        app.world_mut().run_system_once(reset_receipts).unwrap();
        drawn.lock().unwrap().insert(b);
        app.world_mut().run_system_once(submit_receipts).unwrap();
        assert_eq!(*submitted.lock().unwrap(), None, "partial frames must not accumulate");
        drawn.lock().unwrap().insert(a);
        app.world_mut().run_system_once(submit_receipts).unwrap();
        assert_eq!(*submitted.lock().unwrap(), Some(7));
        app.world_mut().run_system_once(reset_receipts).unwrap();
        assert!(app.world().resource::<bevy::render::camera::SortedCameras>().0.is_empty(), "a later pass must not clear the completed atlas");
    }
    #[test]
    fn receipt_requires_all_draws_in_one_frame() {
        let a = Entity::from_bits(1 << 32);
        let b = Entity::from_bits((1 << 32) + 1);
        let expected = HashSet::from([a, b]);
        assert!(!complete_draws(&expected, &HashSet::new()));
        assert!(!complete_draws(&expected, &HashSet::from([a])));
        assert!(!complete_draws(&expected, &HashSet::from([b])));
        assert!(complete_draws(&expected, &expected));
        assert!(!complete_draws(&HashSet::new(), &HashSet::new()));
    }
}
