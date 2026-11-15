//! [design] The mesh terrain blend's bounded 3x3 unlit colour atlas (#2661), using
//! terrain.rs's prepared meshes and materials; nothing is captured, baked or allocated
//! while the blend is off (`mesh_terrain_blend.rs`: the settings row or the test aid).
//! Heights cover every prepared cell; colour validity is per quadrant.
//! A quadrant's colour is its whole near-terrain stack: the base draw (folded or not)
//! and then every blended layer draw over it, as flat unlit copies with the near draws'
//! textures, alpha and draw order (terrain.rs `spawn_terrain_with`). A folded base (only
//! where the GPU has bindless textures, `TerrainMaterials::folds`) is copied with its
//! [`TerrainMaterial`] (terrain.wesl `SURFACE_BAKE`); every other base and every layer
//! with the single-texture [`SurfaceCaptureMaterial`] (terrain_capture.wesl), whose
//! material group binds one texture. The bake publishes only after every copy, opaque
//! and blended, has drawn in one frame. The atlas is 512, 1024 (default) or 2048 texels
//! per cell side (the settings row: 1024; the test aid chooses), checked against the device's texture limit.

use crate::terrain::TerrainMaterial;
use crate::world::{bevy_to_game, game_to_bevy, Streamer, WorldData};
use bevy::asset::RenderAssetUsages;
use bevy::camera::{visibility::RenderLayers, Hdr, RenderTarget, ScalingMode};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::image::ImageSampler;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_phase::{CachedRenderPipelinePhaseItem, DrawFunctions, RenderCommand, RenderCommandResult, RenderCommandState, PhaseItem, TrackedRenderPass};
use bevy::core_pipeline::core_3d::{Opaque3d, Transparent3d};
use bevy::ecs::{query::ROQueryItem, system::SystemParamItem};
use std::{collections::HashSet, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}};
use bevy::render::render_resource::{AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureDimension, TextureFormat, TextureUsages};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::shader::ShaderRef;
use fnv_world::land::SIDE;

pub use nvf_assets::nif_surface_blend::{GRID, HEIGHT_TILE_WIDTH, HEIGHT_TILE_HEIGHT, DEFAULT_RESOLUTION, RESOLUTIONS};
/// Texels across the 3x3-cell colour atlas.
pub fn atlas_width(resolution: u32) -> u32 { 3 * resolution }
/// [design] A resolution the aid accepts and the device can hold as one atlas texture.
pub fn resolution_supported(resolution: u32, max_texture_dimension_2d: u32) -> bool {
    RESOLUTIONS.contains(&resolution) && atlas_width(resolution) <= max_texture_dimension_2d
}
/// Colour atlas storage in bytes (RGBA16F, one mip), CPU copies and render targets aside.
pub fn colour_bytes(resolution: u32) -> u64 { u64::from(atlas_width(resolution)).pow(2) * 8 }
const HEIGHT_WIDTH: usize = 3 * HEIGHT_TILE_WIDTH;
const HEIGHT_HEIGHT: usize = 3 * HEIGHT_TILE_HEIGHT;
pub const BAKE_LAYER: usize = 16;
const INVALID_HEIGHT: f32 = -f32::MAX;

/// [design] How a quadrant's colour is captured: its base draw, folded (bindless) or not
/// (fallback, non-bindless), with `layers` blended layer draws over it; or not at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapturePath { Folded { layers: usize }, Fallback { layers: usize }, HeightOnly(&'static str) }
impl CapturePath {
    fn label(self) -> String {
        let with = |name: &str, layers: usize| if layers == 0 { name.to_string() } else { format!("{name}+{layers}-layers") };
        match self {
            Self::Folded { layers } => with("folded", layers), Self::Fallback { layers } => with("fallback", layers),
            Self::HeightOnly(reason) => format!("height-only:{reason}"),
        }
    }
    fn colour_valid(self) -> bool { !matches!(self, Self::HeightOnly(_)) }
}

pub const CAPTURE_SHADER: &str = "embedded://nvforever/terrain_capture.wesl";

/// [design] The unlit capture of one landscape draw that is not a folded base: a fallback
/// or non-bindless base, or a blended layer (terrain_capture.wesl). One diffuse texture,
/// one sampler, one uniform: its material group fits a GPU without bindless textures,
/// where [`TerrainMaterial`]'s (six standard and thirteen layer textures) does not.
/// Registered in every run, drawn only while the blend bakes; no prepass, no shadows.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[bind_group_data(SurfaceCaptureKey)]
pub struct SurfaceCaptureMaterial {
    #[uniform(0)]
    pub values: SurfaceCaptureUniform,
    /// The source's diffuse texture; `None` samples white (the fallback image), and the
    /// uniform's flag ignores it then.
    #[texture(1)]
    #[sampler(2)]
    pub diffuse: Option<Handle<Image>>,
    /// A layer copy: alpha blended over the copies before it, without depth writes.
    pub layer: bool,
    /// The source material's depth bias: sort order plus rasteriser bias (`sort_first`).
    pub depth_bias: f32,
}

/// [design] terrain_capture.wesl `SurfaceCapture`.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType, Reflect)]
pub struct SurfaceCaptureUniform {
    /// Linear material colour; alpha the material's opacity.
    pub colour: Vec4,
    /// The texture coordinate transform's x axis (xy) and y axis (zw).
    pub uv_axes: Vec4,
    /// xy the transform's translation; z 1 with a diffuse texture; w 1 for a layer.
    pub uv_offset_flags: Vec4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceCaptureKey { layer: bool, raster_bias: i32 }
impl From<&SurfaceCaptureMaterial> for SurfaceCaptureKey {
    fn from(m: &SurfaceCaptureMaterial) -> Self {
        Self { layer: m.layer, raster_bias: nvf_assets::nif_material::raster_bias(m.depth_bias as i32) }
    }
}

impl SurfaceCaptureMaterial {
    /// The capture of a near draw with `source` (terrain.rs `surface`, `layer_material`).
    pub fn from_standard(source: &StandardMaterial, layer: bool) -> Self {
        let colour = LinearRgba::from(source.base_color);
        let t = source.uv_transform;
        Self {
            values: SurfaceCaptureUniform {
                colour: Vec4::new(colour.red, colour.green, colour.blue, colour.alpha),
                uv_axes: Vec4::new(t.matrix2.x_axis.x, t.matrix2.x_axis.y, t.matrix2.y_axis.x, t.matrix2.y_axis.y),
                uv_offset_flags: Vec4::new(t.translation.x, t.translation.y, f32::from(u8::from(source.base_color_texture.is_some())), f32::from(u8::from(layer))),
            },
            diffuse: source.base_color_texture.clone(),
            layer,
            depth_bias: source.depth_bias,
        }
    }
}

impl Material for SurfaceCaptureMaterial {
    fn fragment_shader() -> ShaderRef { CAPTURE_SHADER.into() }
    fn alpha_mode(&self) -> AlphaMode { if self.layer { AlphaMode::Blend } else { AlphaMode::Opaque } }
    /// Orders layer copies as the near layers are ordered (the transparent sort).
    fn depth_bias(&self) -> f32 { self.depth_bias }
    fn enable_prepass() -> bool { false }
    fn enable_shadows() -> bool { false }
    fn specialize(_: &MaterialPipeline, descriptor: &mut RenderPipelineDescriptor, _: &MeshVertexBufferLayoutRef, key: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        // [design] The rasteriser gets only its own part of the bias, as NifSurface's
        // specialize gives the near layer (`raster_bias`).
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.bias.constant = key.bind_group_data.raster_bias;
        }
        Ok(())
    }
}

/// [design] One bake copy's material: a folded base with its own material, or the
/// single-texture capture of any other draw.
#[derive(Clone, Debug)]
pub enum CaptureDraw { Folded(Box<TerrainMaterial>), Single(SurfaceCaptureMaterial) }

#[derive(Component)]
pub struct SurfaceCell {
    pub world: u32,
    pub grid: (i32, i32),
    pub heights: Vec<f32>,
    pub quadrants: Vec<(Mesh, CaptureDraw)>,
    pub paths: [CapturePath; 4],
}

impl SurfaceCell {
    /// A quadrant's draws in their drawing order: its base first, then each blended layer.
    /// Each becomes a flat unlit copy; a layer copy keeps its alpha blending.
    pub fn capture(&mut self, quadrant: u8, draws: Vec<(Mesh, CaptureDraw)>, path: CapturePath) {
        for (mut flat, draw) in draws {
            if let Some(VertexAttributeValues::Float32x3(p)) = flat.attribute_mut(Mesh::ATTRIBUTE_POSITION) {
                for v in p { v[2] = 0.0; }
            }
            let draw = match draw {
                CaptureDraw::Folded(mut material) => {
                    material.extension.surface_bake = true;
                    material.extension.burn = None;
                    material.base.fog_enabled = false;
                    CaptureDraw::Folded(material)
                }
                single => single,
            };
            self.quadrants.push((flat, draw));
        }
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
    pub resolution: u32,
}

struct BakeJob {
    root: Entity,
    publication: PublishedSurface,
    report: String,
}

#[derive(Resource)]
pub struct SurfaceCache {
    pub generation: u64,
    signature: Option<(u32, (i32, i32), Vec<Entity>)>,
    job: Option<BakeJob>,
    ready: Option<u64>,
    submitted: Arc<Mutex<Option<u64>>>,
    pub published: Option<PublishedSurface>,
    /// Colour texels per cell side (the blend's resolution).
    pub resolution: u32,
    /// The device cannot hold the requested atlas ([`check_resolution`]): nothing bakes.
    pub rejected: bool,
}
impl Default for SurfaceCache {
    fn default() -> Self { Self::with_resolution(DEFAULT_RESOLUTION) }
}
impl SurfaceCache {
    pub fn with_resolution(resolution: u32) -> Self {
        Self { generation: 0, signature: None, job: None, ready: None, submitted: default(), published: None, resolution, rejected: false }
    }

    /// [design] The blend turned on: the atlas at `resolution`, checked against the
    /// device's 2D texture limit (`None` without a render device: not checked, not
    /// logged). An unsupported request is rejected in the log and bakes nothing, so no
    /// material samples it. Nothing is allocated here; the first bake allocates.
    pub fn start(&mut self, resolution: u32, max_texture_dimension_2d: Option<u32>) {
        self.resolution = resolution;
        self.rejected = false;
        let Some(max) = max_texture_dimension_2d else { return };
        let width = atlas_width(resolution);
        if resolution_supported(resolution, max) {
            info!("mesh terrain blend: colour atlas resolution={resolution} atlas={width}x{width} rgba16f colour-mib={} max-texture-dimension-2d={max}", colour_bytes(resolution) >> 20);
        } else {
            warn!("mesh terrain blend: rejected resolution={resolution}: atlas {width} texels wide exceeds max-texture-dimension-2d={max}; no colour cache is baked");
            self.rejected = true;
        }
    }

    /// [design] The blend turned off: the pending bake's copies and camera despawned, the
    /// published atlas dropped (its images go with their last handle, the enabled clones'
    /// included), and a new generation, so a receipt of the old bake can never publish.
    /// The generation counter keeps counting across switches.
    pub fn release(&mut self, commands: &mut Commands) {
        if let Some(job) = self.job.take() { commands.entity(job.root).despawn(); }
        self.generation += 1;
        self.signature = None;
        self.ready = None;
        self.published = None;
        *self.submitted.lock().unwrap() = None;
    }
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
pub fn lookup_status(data: &[u8], origin: (i32, i32), point: Vec2, resolution: u32) -> LookupStatus {
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
    let (resolution, width) = (resolution as i32, atlas_width(resolution) as i32);
    let tap = (Vec2::new(atlas.x, 1.0 - atlas.y) * width as f32 - Vec2::splat(0.5)).floor().as_ivec2();
    let residency = height_texel(data, (slot.0 as usize * HEIGHT_TILE_WIDTH, slot.1 as usize * HEIGHT_TILE_HEIGHT + GRID)) as u32;
    for y in 0..2 { for x in 0..2 {
        let pixel = (tap + IVec2::new(x, y)).clamp(IVec2::ZERO, IVec2::splat(width - 1));
        let tap_slot = (pixel.x / resolution, 2 - pixel.y / resolution);
        let offset = (tap_slot.0 - slot.0 + 1, tap_slot.1 - slot.1 + 1);
        let quadrant = u32::from(pixel.x % resolution >= resolution / 2)
            + 2 * u32::from(pixel.y % resolution < resolution / 2);
        if residency & (1 << (offset.1 * 3 + offset.0)) == 0 || !colour_valid(data, tap_slot, quadrant) {
            return LookupStatus::RejectedFootprint;
        }
    }}
    LookupStatus::Success
}

fn publication_log(generation: u64, origin: (i32, i32), resident: &[String], excluded: &[String], probe: &str, assets: (AssetId<Image>, AssetId<Image>), resolution: u32) -> String {
    format!("mesh terrain blend: publication generation={generation} origin=({}, {}) resident=[{}] excluded=[{}] {probe} colour={:?} height={:?} resolution={resolution}", origin.0, origin.1, resident.join(";"), excluded.join(";"), assets.0, assets.1)
}
/// [design] The ground triangle's game-space normal under `point`, from the uploaded
/// coefficients (the `normal` view's colour there is 0.5 + 0.5 n); `None` without heights.
pub fn probe_normal(data: &[u8], origin: (i32, i32), point: Vec2) -> Option<Vec3> {
    let cell = fnv_world::cell_of(point.x, point.y);
    let slot = (cell.0 - origin.0, cell.1 - origin.1);
    if !(0..3).contains(&slot.0) || !(0..3).contains(&slot.1) { return None; }
    let local = point / fnv_world::land::SPACING - Vec2::new(cell.0 as f32, cell.1 as f32) * GRID as f32;
    let square = local.floor().as_uvec2().min(UVec2::splat(GRID as u32 - 1));
    let f = local - square.as_vec2();
    let triangle = usize::from(f.y > f.x);
    let coefficient = |c| {
        let at = height_index(slot, (square.x as usize, square.y as usize), triangle, c);
        f32::from_le_bytes(data[at..at + 4].try_into().unwrap())
    };
    (coefficient(0) > -1e30).then(|| nvf_assets::nif_surface_blend::terrain_game_normal(coefficient(1), coefficient(2)))
}
fn publication_probe(data: &[u8], origin: (i32, i32), point: Vec2, resolution: u32) -> String {
    let cell = fnv_world::cell_of(point.x, point.y);
    let slot = (cell.0 - origin.0, cell.1 - origin.1);
    let local = point / fnv_world::land::SPACING - Vec2::new(cell.0 as f32, cell.1 as f32) * GRID as f32;
    let square = local.floor().as_uvec2().min(UVec2::splat(GRID as u32 - 1));
    let f = local - square.as_vec2();
    let triangle = usize::from(f.y > f.x);
    let at = height_index(slot, (square.x as usize, square.y as usize), triangle, 0);
    let normal = probe_normal(data, origin, point).map_or("none".to_string(), |n| format!("({:.3}, {:.3}, {:.3})", n.x + 0.0, n.y + 0.0, n.z + 0.0));
    format!("probe-cell={cell:?} probe-slot={slot:?} probe-game-xy={point:?} probe-square={square:?} probe-triangle={triangle} probe-height-bytes={:?} probe-status={:?} probe-normal={normal}", &data[at..at + 4], lookup_status(data, origin, point, resolution))
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

/// [design] Registered whether or not the blend is on, so the settings row can switch it
/// while the game runs; while it is off no bake view exists, so none of this draws, and
/// the draw functions stay Bevy's own until the first bake view is extracted.
pub struct SurfaceBakePlugin;
impl Plugin for SurfaceBakePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "terrain_capture.wesl");
        app.add_plugins((ExtractComponentPlugin::<BakeView>::default(), MaterialPlugin::<SurfaceCaptureMaterial>::default()));
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.add_systems(Render, install_bake_receipts.run_if(first_bake_view).in_set(RenderSystems::Prepare))
                .add_systems(Render, reset_receipts.in_set(RenderSystems::Prepare))
                .add_systems(Render, submit_receipts.in_set(RenderSystems::Cleanup))
                .add_systems(bevy::core_pipeline::Core3d, (
                    bake_span_begin.before(bevy::core_pipeline::Core3dSystems::Prepass),
                    bake_span_end.after(bevy::core_pipeline::Core3dSystems::PostProcess),
                ).run_if(any_bake_view));
        }
    }
}

/// A bake view is in the render world (the span systems run only then).
fn any_bake_view(views: Query<(), With<BakeView>>) -> bool {
    !views.is_empty()
}

/// The render world's note that the receipts wrap the draw functions now.
#[derive(Resource)]
struct BakeReceiptsInstalled;

/// [design] A bake view is in the render world and the receipts are not installed yet:
/// the blend has just baked for the first time this run (a run that never turns the blend
/// on keeps Bevy's own draw functions).
fn first_bake_view(views: Query<(), With<BakeView>>, installed: Option<Res<BakeReceiptsInstalled>>) -> bool {
    installed.is_none() && !views.is_empty()
}

/// Both phases' receipts, once (before the bake's passes are recorded; `DrawFunctions`
/// prepares each function when its phase renders).
fn install_bake_receipts(world: &mut World) {
    install_bake_receipt::<Opaque3d>(world);
    install_bake_receipt::<Transparent3d>(world);
    world.insert_resource(BakeReceiptsInstalled);
}

// [design] Base copies draw in Opaque3d, layer copies (alpha blended) in Transparent3d;
// both phases' DrawMaterial carries the receipt.
fn install_bake_receipt<P: CachedRenderPipelinePhaseItem>(world: &mut World) {
    let command = RenderCommandState::<P, (bevy::pbr::DrawMaterial, BakeReceipt)>::new(world);
    let functions = world.resource::<DrawFunctions<P>>();
    let mut functions = functions.write();
    let id = functions.id::<bevy::pbr::DrawMaterial>();
    functions.draw_functions[id.0 as usize] = Box::new(command);
}

/// The GPU time span's name (Tracy's GPU zone and the `--log-frame-times` list, graphics.rs).
pub const BAKE_SPAN: &str = "nvf_mesh_terrain_bake";
/// What the bake's span systems read: the render context, the view being drawn and the
/// bake views.
type BakeSpanParams = (bevy::render::renderer::RenderContext<'static, 'static>, Option<Res<'static, bevy::render::renderer::CurrentView>>, Query<'static, 'static, (), With<BakeView>>);

/// [design] A GPU time span around a bake view's whole draw ([`BAKE_SPAN`]; recorded only
/// with the render diagnostics on: `--log-frame-times`, a Tracy build). The span opens
/// before the view's first pass and closes after its last, so the bake's own passes nest
/// in it. Both ends are exclusive systems: the executor runs those on its scope's thread,
/// and the recorder keeps open spans per thread, so the end finds the span the begin
/// opened. Any other view, or no recorder: nothing.
fn bake_span_begin(world: &mut World, state: &mut bevy::ecs::system::SystemState<BakeSpanParams>) {
    bake_span(world, state, true);
}
fn bake_span_end(world: &mut World, state: &mut bevy::ecs::system::SystemState<BakeSpanParams>) {
    bake_span(world, state, false);
}
fn bake_span(world: &mut World, state: &mut bevy::ecs::system::SystemState<BakeSpanParams>, begin: bool) {
    use bevy::render::diagnostic::RecordDiagnostics;
    {
        let Ok((mut ctx, view, bakes)) = state.get_mut(world) else { return };
        if !view.is_some_and(|v| bakes.contains(v.0)) { return; }
        let Some(recorder) = ctx.diagnostic_recorder() else { return };
        let encoder = ctx.command_encoder();
        if begin {
            recorder.begin_time_span(encoder, BAKE_SPAN.into());
        } else {
            recorder.end_time_span(encoder);
        }
    }
    state.apply(world);
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
impl<P: PhaseItem> RenderCommand<P> for BakeReceipt {
    type Param = ();
    type ViewQuery = Option<&'static BakeView>;
    type ItemQuery = ();
    fn render<'w>(item: &P, view: ROQueryItem<'w, '_, Self::ViewQuery>, _: Option<()>, _: SystemParamItem<'w, '_, Self::Param>, _: &mut TrackedRenderPass<'w>) -> RenderCommandResult {
        if let Some(view) = view {
            // NoAutomaticBatching + NoIndirectDrawing: one successful indexed draw
            // represents this exact quadrant, with no indirect culling ambiguity.
            view.drawn.lock().unwrap().insert(item.main_entity().id());
        }
        RenderCommandResult::Success
    }
}

/// [design] Cells drawn while the blend was off have no [`SurfaceCell`]: each loaded
/// exterior cell of the 3x3 region that lacks one gets its capture from its landscape
/// prepared again (`terrain.rs::capture_surface`, the same quadrant stacks and cached
/// materials as a cell drawn with the blend on), once per cell entity while the blend
/// stays on (`BlendState::tried`, emptied when it turns off). Cells loaded while it is on
/// are captured as they are drawn.
#[allow(clippy::too_many_arguments)]
pub fn backfill_cells(
    mut commands: Commands,
    world: Res<WorldData>,
    streamer: Res<Streamer>,
    camera: Query<&GlobalTransform, crate::fly::WorldCamera>,
    captured: Query<(), With<SurfaceCell>>,
    mut terrain: crate::terrain::TerrainMaterials,
    mut models: nvf_assets::model::ModelCtx,
    mut state: ResMut<crate::mesh_terrain_blend::BlendState>,
) {
    if streamer.interior.is_some() { return; }
    let Ok(camera) = camera.single() else { return };
    let origin = cache_origin(bevy_to_game(camera.translation()).truncate());
    for y in origin.1..origin.1 + 3 { for x in origin.0..origin.0 + 3 {
        let Some(&cell) = streamer.loaded.get(&(x, y)) else { continue };
        if captured.contains(cell) || !state.tried.insert(cell) { continue; }
        let Some(land) = world.grid_cell((x, y)).and_then(|c| c.land) else { continue };
        let tile = terrain.settings.tile;
        let Some(prep) = crate::terrain::prepare_terrain(world.lo, world.schema, land, (x, y), tile, world.neighbour_lands((x, y))) else { continue };
        let folds = terrain.folds();
        if let Some(surface) = crate::terrain::capture_surface(&world, prep, &mut crate::terrain::TerrainCtx { tm: &mut terrain, assets: &mut models }, folds) {
            commands.entity(cell).try_insert(surface);
        }
    }}
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
    mut captures: ResMut<Assets<SurfaceCaptureMaterial>>,
) {
    if cache.rejected { return; }
    let resolution = cache.resolution;
    let width = atlas_width(resolution);
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
        Extent3d { width, height: width, depth_or_array_layers: 1 },
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
        for (mesh, draw) in &cell.quadrants {
            let mut entity = commands.spawn((
                bevy::render::batching::NoAutomaticBatching, Mesh3d(meshes.add(mesh.clone())),
                Transform::from_xyz(slot.0 as f32 * 4096.0, slot.1 as f32 * 4096.0, 0.0),
                RenderLayers::layer(BAKE_LAYER), bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver, ChildOf(root),
            ));
            match draw {
                CaptureDraw::Folded(material) => entity.insert(MeshMaterial3d(materials.add(material.as_ref().clone()))),
                CaptureDraw::Single(material) => entity.insert(MeshMaterial3d(captures.add(material.clone()))),
            };
            expected.insert(entity.id());
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
    let probe = publication_probe(&heights, origin, eye.truncate(), resolution);
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
    let report = publication_log(cache.generation, origin, &resident_report, &excluded, &probe, (colour.id(), height.id()), resolution);
    cache.job = Some(BakeJob {
        root,
        publication: PublishedSurface { colour, height, origin, cells: resident.len(), resolution },
        report,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn surface(paths: [CapturePath; 4]) -> SurfaceCell {
        SurfaceCell { world: 0, grid: (-17, 0), heights: vec![42.5; SIDE * SIDE], quadrants: vec![], paths }
    }
    /// The probe's classification, the same at every accepted resolution.
    fn status(data: &[u8], origin: (i32, i32), point: Vec2) -> LookupStatus {
        let [a, b, c] = RESOLUTIONS.map(|r| lookup_status(data, origin, point, r));
        assert!(a == b && b == c, "{point:?}: {a:?} {b:?} {c:?}");
        a
    }
    #[test]
    fn footprint_boundary_scales_with_resolution_in_negative_cells_too() {
        // q0 of slot (0, 0) has no colour. A point 3 units east of the quadrant edge: at 8
        // units per texel its bilinear footprint still reaches the invalid quadrant, at 4
        // and 2 units it does not. The same holds with every coordinate shifted to
        // negative cells.
        let mut data = empty_height_atlas();
        let cell = surface([CapturePath::Folded { layers: 0 }; 4]);
        let slots = HashSet::from([(0, 0), (1, 0), (0, 1), (1, 1)]);
        for &slot in &slots { encode_surface_cell(&mut data, slot, &cell, &slots); }
        encode_colour_mask(&mut data, (0, 0), 0b1110);
        for origin in [(0, 0), (-3, -2), (-18, -1)] {
            let shift = Vec2::new(origin.0 as f32, origin.1 as f32) * 4096.0;
            let point = shift + Vec2::new(2051.0, 1024.0);
            assert_eq!(lookup_status(&data, origin, point, 512), LookupStatus::RejectedFootprint, "{origin:?}");
            assert_eq!(lookup_status(&data, origin, point, 1024), LookupStatus::Success, "{origin:?}");
            assert_eq!(lookup_status(&data, origin, point, 2048), LookupStatus::Success, "{origin:?}");
            // One texel from the cell edge at each resolution, towards the resident east cell.
            for resolution in RESOLUTIONS {
                let texel = 4096.0 / resolution as f32;
                assert_eq!(lookup_status(&data, origin, shift + Vec2::new(4096.0 - 0.25 * texel, 1024.0), resolution), LookupStatus::Success, "{origin:?} {resolution}");
                assert_eq!(lookup_status(&data, origin, shift + Vec2::new(0.25 * texel, 3072.0), resolution), LookupStatus::Success, "{origin:?} {resolution}: taps clamp at the atlas's west edge (the shader's border fade covers it)");
            }
            // The missing quadrant itself, at every resolution.
            assert_eq!(status(&data, origin, shift + Vec2::splat(1024.0)), LookupStatus::InvalidColourQuadrant);
        }
    }
    #[test]
    fn resolutions_are_the_three_accepted_and_checked_against_the_device_limit() {
        assert_eq!(RESOLUTIONS, [512, 1024, 2048]);
        assert_eq!(DEFAULT_RESOLUTION, 1024);
        assert_eq!(SurfaceCache::default().resolution, 1024);
        assert_eq!(RESOLUTIONS.map(atlas_width), [1536, 3072, 6144]);
        assert_eq!(RESOLUTIONS.map(|r| colour_bytes(r) >> 20), [18, 72, 288], "MiB of the RGBA16F atlas");
        assert!(resolution_supported(2048, 8192) && resolution_supported(2048, 6144));
        assert!(!resolution_supported(2048, 6143) && !resolution_supported(1024, 2048));
        assert!(resolution_supported(512, 2048));
        assert!(!resolution_supported(768, 16384), "only the listed resolutions");
    }
    #[test]
    fn unsupported_quadrants_and_height_only_cells_keep_all_prepared_heights() {
        let mut data = empty_height_atlas();
        let slots = HashSet::from([(0, 0), (1, 0)]);
        let partial = surface([CapturePath::Folded { layers: 0 }, CapturePath::HeightOnly("missing-base-mesh"), CapturePath::Fallback { layers: 2 }, CapturePath::Folded { layers: 1 }]);
        let height_only = surface([CapturePath::HeightOnly("missing-base-mesh"); 4]);
        encode_surface_cell(&mut data, (0, 0), &partial, &slots);
        encode_surface_cell(&mut data, (1, 0), &height_only, &slots);
        assert_eq!(partial.colour_mask(), 0b1101);
        assert_eq!(height_only.colour_mask(), 0);
        for slot in [(0, 0), (1, 0)] { for y in 0..GRID { for x in 0..GRID { for triangle in 0..2 {
            let at = height_index(slot, (x, y), triangle, 0);
            assert_eq!(f32::from_le_bytes(data[at..at + 4].try_into().unwrap()), 42.5);
        }}}}
        assert_eq!(status(&data, (-18, -1), Vec2::new(-18.0 * 4096.0 + 1024.0, -3072.0)), LookupStatus::Success);
        assert_eq!(status(&data, (-18, -1), Vec2::new(-18.0 * 4096.0 + 3072.0, -3072.0)), LookupStatus::InvalidColourQuadrant);
        assert_eq!(status(&data, (-18, -1), Vec2::new(-17.0 * 4096.0 + 1024.0, -3072.0)), LookupStatus::InvalidColourQuadrant);
    }
    #[test]
    fn cpu_publication_probe_classifies_all_five_shader_outcomes() {
        let mut data = empty_height_atlas();
        let cell = surface([CapturePath::Folded { layers: 0 }; 4]);
        let slots = HashSet::from([(0, 0), (1, 0), (0, 1)]);
        for &slot in &slots { encode_surface_cell(&mut data, slot, &cell, &slots); }
        assert_eq!(status(&data, (0, 0), Vec2::new(-1.0, 100.0)), LookupStatus::OutOfRange);
        assert_eq!(status(&data, (0, 0), Vec2::splat(6144.0)), LookupStatus::InvalidHeight);
        assert_eq!(status(&data, (0, 0), Vec2::splat(4095.9)), LookupStatus::RejectedFootprint, "missing diagonal tap");
        assert_eq!(status(&data, (0, 0), Vec2::splat(1024.0)), LookupStatus::Success);
        encode_colour_mask(&mut data, (0, 0), 0b1110);
        assert_eq!(status(&data, (0, 0), Vec2::splat(1024.0)), LookupStatus::InvalidColourQuadrant);
        assert_eq!(status(&data, (0, 0), Vec2::new(2048.1, 1024.0)), LookupStatus::RejectedFootprint, "one bilinear tap crosses into the invalid neighbour quadrant");
        assert_eq!(status(&data, (0, 0), Vec2::new(3072.0, 1024.0)), LookupStatus::Success, "supported neighbour quadrant is retained");
    }
    #[test]
    fn publication_line_names_paths_exclusions_probe_and_asset_ids() {
        let id = Handle::<Image>::default().id();
        let resident = vec![format!("(-17, 0):q=[{}] colour-mask=0x7", [CapturePath::Folded { layers: 0 }, CapturePath::Folded { layers: 2 }, CapturePath::Fallback { layers: 3 }, CapturePath::HeightOnly("missing-base-mesh")].map(CapturePath::label).join(","))];
        let line = publication_log(7, (-18, -1), &resident, &["(-18, -1):not-loaded".into()], "probe-slot=(1, 1)", (id, id), 1024);
        assert_eq!(line, format!("mesh terrain blend: publication generation=7 origin=(-18, -1) resident=[(-17, 0):q=[folded,folded+2-layers,fallback+3-layers,height-only:missing-base-mesh] colour-mask=0x7] excluded=[(-18, -1):not-loaded] probe-slot=(1, 1) colour={id:?} height={id:?} resolution=1024"));
        let mut data = empty_height_atlas();
        let cell = surface([CapturePath::Folded { layers: 0 }; 4]);
        encode_surface_cell(&mut data, (1, 1), &cell, &HashSet::from([(1, 1)]));
        let probe = publication_probe(&data, (-18, -1), Vec2::new(-17.0 * 4096.0 + 1024.0, 1024.0), 1024);
        assert!(probe.contains("probe-cell=(-17, 0) probe-slot=(1, 1)"));
        assert!(probe.contains(&format!("probe-height-bytes={:?} probe-status=Success probe-normal=(0.000, 0.000, 1.000)", 42.5f32.to_le_bytes())), "{probe}");
        assert!(publication_probe(&empty_height_atlas(), (-18, -1), Vec2::new(-17.0 * 4096.0 + 1024.0, 1024.0), 512).contains("probe-status=InvalidHeight probe-normal=none"));
    }
    /// [design] The probe's normal comes from the uploaded coefficients of the triangle
    /// under the point, both triangles of a square, in game space; Bevy world space is
    /// `game_to_bevy`'s rotation of it.
    #[test]
    fn probe_normal_reads_both_triangles_and_converts_like_the_world_root() {
        let spacing = fnv_world::land::SPACING;
        // Heights rising 1 per unit east and 0.5 per unit north, plus a kink so the two
        // triangles of each square differ: the north-east corner of every square is raised.
        let h: Vec<f32> = (0..SIDE * SIDE).map(|i| (i % SIDE) as f32 * spacing + (i / SIDE) as f32 * spacing * 0.5 + if (i % SIDE) % 2 == 1 && (i / SIDE) % 2 == 1 { 64.0 } else { 0.0 }).collect();
        let mut data = empty_height_atlas();
        encode_height_cell(&mut data, (1, 1), &h);
        let origin = (-18, -3);
        let square = Vec2::new(-17.0 * 4096.0, -2.0 * 4096.0);
        for (f, triangle) in [(Vec2::new(0.7, 0.2), 0), (Vec2::new(0.2, 0.7), 1)] {
            let n = probe_normal(&data, origin, square + f * spacing).unwrap();
            let (a, east, north, ne) = (h[0], h[1], h[SIDE], h[SIDE + 1]);
            let corners = if triangle == 0 {
                [Vec3::new(0.0, 0.0, a), Vec3::new(spacing, 0.0, east), Vec3::new(spacing, spacing, ne)]
            } else {
                [Vec3::new(0.0, 0.0, a), Vec3::new(spacing, spacing, ne), Vec3::new(0.0, spacing, north)]
            };
            assert!(n.dot(corners[1] - corners[0]).abs() < 1e-3 && n.dot(corners[2] - corners[0]).abs() < 1e-3, "triangle {triangle}: {n:?}");
            let world = nvf_assets::nif_surface_blend::game_to_world_normal(n);
            assert!((world - crate::world::game_to_bevy().rotation * n).length() < 1e-5);
            assert!(world.y > 0.0, "up in Bevy space");
        }
        assert_ne!(probe_normal(&data, origin, square + Vec2::new(0.7, 0.2) * spacing), probe_normal(&data, origin, square + Vec2::new(0.2, 0.7) * spacing));
        assert_eq!(probe_normal(&data, origin, square - Vec2::splat(4096.0 * 2.0)), None, "outside the atlas");
        assert_eq!(probe_normal(&data, (-17, -2), square), None, "slot without heights");
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
    /// [design] Production path regression: `spawn_terrain_with` captures every quadrant's
    /// whole draw stack (folded or fallback base plus the blended layers drawn over it) and
    /// `update_cache` publishes the cell's complete heights with all four colour quadrants
    /// valid, waiting for one receipt per near draw. A return to whole-cell or
    /// per-layered-quadrant exclusion fails here. Synthetic land and a synthetic BC1
    /// texture; no game data.
    #[test]
    fn spawned_layered_quadrants_publish_whole_heights_full_colour_and_every_draw() {
        use bevy::ecs::system::RunSystemOnce;
        use fnv_world::land::{Land, Layer, QUAD_SIDE};
        let grid = (-17, 0);
        let heights: Vec<f32> = (0..SIDE * SIDE).map(|i| ((i * 13 + i / SIDE * 7) % 59) as f32 * 2.5 - 40.0).collect();
        // Layers paint only the quadrant's middle vertex, so stitching adds none across
        // the quadrant edges: q0 has 7 layers (6 fold, 1 stays blended where folding is
        // on), q1 2, q2 none, q3 no base record and no layers.
        let centre = ((QUAD_SIDE / 2) * QUAD_SIDE + QUAD_SIDE / 2) as u16;
        let mut layers = Vec::new();
        for (q, count) in [(0u8, 7i16), (1, 2), (2, 0)] {
            layers.push(Layer { quadrant: q, texture: None, layer: -1, base: true, alpha: vec![] });
            for layer in 0..count {
                layers.push(Layer { quadrant: q, texture: None, layer, base: false, alpha: vec![(centre, 0.75)] });
            }
        }
        let land = Land { heights: Some(heights.clone()), layers, ..default() };
        for (folds, expected) in [
            (true, [CapturePath::Folded { layers: 1 }, CapturePath::Folded { layers: 0 }, CapturePath::Fallback { layers: 0 }, CapturePath::Fallback { layers: 0 }]),
            (false, [CapturePath::Fallback { layers: 7 }, CapturePath::Fallback { layers: 2 }, CapturePath::Fallback { layers: 0 }, CapturePath::Fallback { layers: 0 }]),
        ] {
            let (mut app, _files) = crate::outfits::tests::asset_fixture();
            app.insert_resource(crate::outfits::tests::empty_world_data())
                .insert_resource(crate::terrain::TerrainSettings { tile: 256.0 })
                .init_resource::<crate::terrain::TerrainCache>()
                .init_resource::<Assets<TerrainMaterial>>()
                .init_resource::<Assets<SurfaceCaptureMaterial>>()
                .init_resource::<SurfaceCache>();
            app.world_mut().resource_mut::<nvf_assets::TextureCache>()
                .offer(fnv_vfs::texture_path(fnv_world::land::DEFAULT_DIFFUSE), synthetic_dds(), 0.0);
            let cell = app.world_mut().spawn_empty().id();
            let prep_land = land.clone();
            app.world_mut().run_system_once(move |mut commands: Commands, world: Res<WorldData>, mut terrain: crate::terrain::TerrainMaterials, mut models: nvf_assets::model::ModelCtx| {
                let prep = crate::terrain::prepare_land(prep_land.clone(), &[None, None, None, None], grid, 256.0);
                crate::terrain::spawn_terrain_with(&mut commands, cell, &world, prep, &mut crate::terrain::TerrainCtx { tm: &mut terrain, assets: &mut models }, folds, true);
            }).unwrap();
            let near_draws = app.world_mut().query::<(&ChildOf, &crate::terrain::LandMesh)>().iter(app.world()).filter(|(parent, _)| parent.parent() == cell).count();
            // Round 7: the capture of a cell drawn while the blend was off (the settings row
            // turned on later) is the same stack, and draws nothing near again.
            let again = land.clone();
            let captured = app.world_mut().run_system_once(move |world: Res<WorldData>, mut terrain: crate::terrain::TerrainMaterials, mut models: nvf_assets::model::ModelCtx| {
                let prep = crate::terrain::prepare_land(again.clone(), &[None, None, None, None], grid, 256.0);
                crate::terrain::capture_surface(&world, prep, &mut crate::terrain::TerrainCtx { tm: &mut terrain, assets: &mut models }, folds)
            }).unwrap().expect("heights give a capture");
            let land_meshes = app.world_mut().query::<&crate::terrain::LandMesh>().iter(app.world()).count();
            assert_eq!(land_meshes, near_draws, "folds={folds}: the capture alone spawns no near draw");
            let surface = app.world().get::<SurfaceCell>(cell).expect("capture inserts the cell's surface");
            assert_eq!((captured.paths, captured.heights.len(), captured.quadrants.len()), (surface.paths, surface.heights.len(), surface.quadrants.len()), "folds={folds}");
            let kind = |d: &CaptureDraw| match d { CaptureDraw::Folded(m) => (true, false, m.base.depth_bias.to_bits()), CaptureDraw::Single(m) => (false, m.layer, m.depth_bias.to_bits()) };
            assert!(captured.quadrants.iter().zip(&surface.quadrants).all(|((_, a), (_, b))| kind(a) == kind(b)), "folds={folds}: the same copies in the same order");
            assert_eq!(surface.paths, expected, "folds={folds}");
            assert_eq!(surface.colour_mask(), 0xf, "folds={folds}: every quadrant's colour is captured");
            assert_eq!(surface.quadrants.len(), near_draws, "folds={folds}: one bake copy per near draw");
            let blended_layers: usize = expected.iter().map(|p| match p { CapturePath::Folded { layers } | CapturePath::Fallback { layers } => *layers, CapturePath::HeightOnly(_) => 0 }).sum();
            // Folded bases keep their TerrainMaterial (unlit bake variant); every other base
            // and every layer is a single-texture capture, so a device without bindless
            // (folds=false) never meets TerrainMaterial's 19-texture layout (round-4 P2).
            let folded: Vec<_> = surface.quadrants.iter().filter_map(|(_, d)| match d { CaptureDraw::Folded(m) => Some(m), CaptureDraw::Single(_) => None }).collect();
            let single: Vec<_> = surface.quadrants.iter().filter_map(|(_, d)| match d { CaptureDraw::Single(m) => Some(m), CaptureDraw::Folded(_) => None }).collect();
            let folded_paths = expected.iter().filter(|p| matches!(p, CapturePath::Folded { .. })).count();
            assert_eq!(folded.len(), folded_paths, "folds={folds}: one folded copy per folded base, none without folding");
            assert!(folded.iter().all(|m| m.extension.surface_bake && m.extension.burn.is_none() && !m.base.fog_enabled), "folded copies use the unlit bake variant");
            assert_eq!(single.iter().filter(|m| m.layer).count(), blended_layers, "folds={folds}");
            assert!(single.iter().filter(|m| m.layer).all(|m| m.alpha_mode() == AlphaMode::Blend && m.values.uv_offset_flags.w == 1.0), "layer copies keep the near layers' blending");
            assert!(single.iter().filter(|m| !m.layer).all(|m| m.alpha_mode() == AlphaMode::Opaque && m.depth_bias == 0.0), "base copies stay opaque");
            assert!(single.iter().all(|m| m.diffuse.is_some() && m.values.uv_offset_flags.z == 1.0), "the synthetic default texture is bound");
            // Layer copies keep the near layers' sort order and the rasteriser part of their bias.
            // In drawing order, each quadrant's layer copies follow its base with rising bias.
            let mut previous: Option<f32> = None;
            for (_, draw) in &surface.quadrants {
                match draw {
                    CaptureDraw::Single(m) if m.layer => {
                        assert!(previous.is_some_and(|p| m.depth_bias > p), "folds={folds}: a layer copy follows its base or a lower layer");
                        previous = Some(m.depth_bias);
                    }
                    _ => previous = Some(f32::NEG_INFINITY),
                }
            }
            let biases: Vec<_> = single.iter().filter(|m| m.layer).map(|m| (m.depth_bias, SurfaceCaptureKey::from(*m).raster_bias)).collect();
            assert!(biases.iter().all(|&(sort, raster)| nvf_assets::nif_material::sorts_first(sort) && raster == nvf_assets::nif_material::raster_bias(sort as i32)), "{biases:?}");
            assert!(surface.quadrants.iter().all(|(mesh, _)| match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
                Some(VertexAttributeValues::Float32x3(p)) => p.iter().all(|v| v[2] == 0.0),
                _ => false,
            }), "copies are flat");

            // The publication: the production cache update around a camera in the cell.
            app.insert_resource(Streamer {
                radius: 1, loaded: [(grid, cell)].into(), fades: default(), fade_secs: 0.0, settled: true,
                arrival: None, per_frame: 1, spawning: default(), dying: default(), ahead: default(),
                hide: default(), interior: None, light_scale: 1.0, managed: default(), manage_actors: false,
                stats: default(), buffer: default(), centre: None, waiting: default(), model_overrides: default(),
            });
            let eye = Vec3::new(grid.0 as f32 * 4096.0 + 2048.0, grid.1 as f32 * 4096.0 + 2048.0, 600.0);
            app.world_mut().spawn((Camera3d::default(), crate::fly::FlyCam { yaw: 0.0, pitch: 0.0, speed: 0.0 }, GlobalTransform::from_translation(Vec3::new(eye.x, eye.z, -eye.y))));
            app.world_mut().run_system_once(update_cache).unwrap();
            let cache = app.world().resource::<SurfaceCache>();
            let job = cache.job.as_ref().expect("a bake is pending for the resident cell");
            assert_eq!((job.publication.origin, job.publication.cells), ((grid.0 - 1, grid.1 - 1), 1));
            assert!(job.report.contains("colour-mask=0xf"), "{}", job.report);
            let height = app.world().resource::<Assets<Image>>().get(&job.publication.height).unwrap().data.clone().unwrap();
            let origin = job.publication.origin;
            for y in 0..GRID { for x in 0..GRID { for triangle in 0..2 {
                let at = height_index((1, 1), (x, y), triangle, 0);
                assert_eq!(f32::from_le_bytes(height[at..at + 4].try_into().unwrap()), heights[y * SIDE + x], "square ({x}, {y})");
            }}}
            for q in 0..4 {
                let point = Vec2::new(grid.0 as f32 * 4096.0 + 1024.0 + 2048.0 * (q % 2) as f32, grid.1 as f32 * 4096.0 + 1024.0 + 2048.0 * (q / 2) as f32);
                assert_eq!(status(&height, origin, point), LookupStatus::Success, "folds={folds} quadrant {q}");
            }
            let expected_draws = app.world_mut().query::<&BakeView>().single(app.world()).unwrap().expected.len();
            assert_eq!(expected_draws, near_draws, "folds={folds}: the receipt waits for every copy, blended layers included");
            let terrain_copies = app.world_mut().query_filtered::<(), (With<RenderLayers>, With<MeshMaterial3d<TerrainMaterial>>)>().iter(app.world()).count();
            let single_copies = app.world_mut().query_filtered::<(), (With<RenderLayers>, With<MeshMaterial3d<SurfaceCaptureMaterial>>)>().iter(app.world()).count();
            assert_eq!((terrain_copies, single_copies), (folded_paths, near_draws - folded_paths), "folds={folds}: bake entities by material type");
            let colour = app.world().resource::<Assets<Image>>().get(&app.world().resource::<SurfaceCache>().job.as_ref().unwrap().publication.colour).unwrap();
            assert_eq!(colour.texture_descriptor.size.width, atlas_width(DEFAULT_RESOLUTION), "the default 1024-texel cells");
        }
    }
    #[test]
    fn rejected_resolution_bakes_and_publishes_nothing() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        app.insert_resource(crate::outfits::tests::empty_world_data())
            .init_resource::<Assets<Image>>().init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<TerrainMaterial>>().init_resource::<Assets<SurfaceCaptureMaterial>>()
            .insert_resource(SurfaceCache { rejected: true, ..SurfaceCache::with_resolution(2048) });
        let cell = app.world_mut().spawn(SurfaceCell { world: 0, grid: (0, 0), heights: vec![0.0; SIDE * SIDE], quadrants: vec![], paths: [CapturePath::Fallback { layers: 0 }; 4] }).id();
        app.insert_resource(Streamer {
            radius: 1, loaded: [((0, 0), cell)].into(), fades: default(), fade_secs: 0.0, settled: true,
            arrival: None, per_frame: 1, spawning: default(), dying: default(), ahead: default(),
            hide: default(), interior: None, light_scale: 1.0, managed: default(), manage_actors: false,
            stats: default(), buffer: default(), centre: None, waiting: default(), model_overrides: default(),
        });
        app.world_mut().spawn((Camera3d::default(), crate::fly::FlyCam { yaw: 0.0, pitch: 0.0, speed: 0.0 }, GlobalTransform::from_translation(Vec3::new(2048.0, 600.0, -2048.0))));
        for _ in 0..3 { app.world_mut().run_system_once(update_cache).unwrap(); }
        let cache = app.world().resource::<SurfaceCache>();
        assert!(cache.job.is_none() && cache.published.is_none() && cache.generation == 0);
        assert!(app.world().resource::<Assets<Image>>().is_empty());
    }
    fn synthetic_dds() -> Vec<u8> {
        // A 4x4 BC1 DDS header and one block, as the texture-swap test's.
        let mut dds = vec![0u8; 136];
        dds[..4].copy_from_slice(b"DDS ");
        for (at, value) in [(4, 124u32), (8, 0x1007), (12, 4), (16, 4), (28, 1), (76, 32), (80, 4), (108, 0x1000)] { dds[at..at + 4].copy_from_slice(&value.to_le_bytes()); }
        dds[84..88].copy_from_slice(b"DXT1");
        dds
    }
    #[test]
    fn receipt_requires_all_draws_in_one_frame() {
        let mut world = World::new();
        let a = world.spawn_empty().id();
        let b = world.spawn_empty().id();
        let expected = HashSet::from([a, b]);
        assert!(!complete_draws(&expected, &HashSet::new()));
        assert!(!complete_draws(&expected, &HashSet::from([a])));
        assert!(!complete_draws(&expected, &HashSet::from([b])));
        assert!(complete_draws(&expected, &expected));
        assert!(!complete_draws(&HashSet::new(), &HashSet::new()));
    }
}
