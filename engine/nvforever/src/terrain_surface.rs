//! [design] The mesh terrain blend's bounded 3x3 unlit colour atlas (#2661), using
//! terrain.rs's prepared meshes and materials; nothing is captured, baked or allocated
//! while the blend is off (`mesh_terrain_blend.rs`: the settings row or the test aid;
//! [`SurfaceBakePlugin`] says what runs then).
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
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderSystems, sync_world::RenderEntity};
use bevy::render::render_phase::{CachedRenderPipelinePhaseItem, DrawFunctions, RenderCommand, RenderCommandResult, RenderCommandState, PhaseItem, TrackedRenderPass};
use bevy::core_pipeline::core_3d::{Opaque3d, Transparent3d};
use bevy::ecs::{query::ROQueryItem, system::SystemParamItem};
use std::{collections::{HashMap, HashSet}, sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}}};
use bevy::render::render_resource::{AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureDimension, TextureFormat, TextureUsages};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialGate, MaterialPipeline, MaterialPipelineKey};
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
/// Registered in every run behind the blend's gate ([`SurfaceBakePlugin`]), drawn only
/// while the blend bakes; no prepass, no shadows.
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

/// [design] A new reconstruction gets a new input revision, including stitched inputs.
pub fn capture_revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone)]
pub enum CaptureSource { Folded(Handle<TerrainMaterial>), Single(Handle<nvf_assets::NifMaterial>, bool) }

#[derive(Component)]
pub struct SurfaceCell {
    pub world: u32,
    pub grid: (i32, i32),
    pub heights: Vec<f32>,
    pub quadrants: Vec<(Mesh, CaptureDraw)>,
    pub paths: [CapturePath; 4],
    pub content_revision: u64,
    pub sources: Vec<CaptureSource>,
    /// Diffuse dependencies recorded with the captured draw stack, never mesh hashes.
    pub dependencies: HashSet<AssetId<Image>>,
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
        self.refresh_dependencies();
    }
    fn image_dependencies(&self) -> &HashSet<AssetId<Image>> { &self.dependencies }
    fn refresh_dependencies(&mut self) {
        let mut images = HashSet::new();
        for (_, draw) in &self.quadrants {
            match draw {
                CaptureDraw::Single(m) => { images.extend(m.diffuse.as_ref().map(Handle::id)); }
                CaptureDraw::Folded(m) => {
                    let e = &m.extension;
                    for h in [&m.base.base_color_texture, &e.diffuse0, &e.diffuse1, &e.diffuse2, &e.diffuse3, &e.diffuse4, &e.diffuse5].into_iter().flatten() { images.insert(h.id()); }
                }
            }
        }
        self.dependencies = images;
    }
    fn colour_mask(&self) -> u32 {
        self.paths.iter().enumerate().fold(0, |mask, (q, path)| mask | (u32::from(path.colour_valid()) << q))
    }
}

#[derive(Clone, Default)]
pub struct PublishedSurface {
    pub colour: Handle<Image>,
    pub height: Handle<Image>,
    pub origin: (i32, i32),
    pub cells: usize,
    pub resolution: u32,
    pub revision: u64,
    /// Enabled-only mask in this publication's original coordinates.
    pub coverage: u32,
    pub(crate) keys: Vec<CellKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CellKey { grid: (i32, i32), entity: Entity, revision: u64, changed: u32, images: Vec<(AssetId<Image>, u64)> }

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request { epoch: u64, origin: (i32, i32), cells: Vec<CellKey> }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Receipt { epoch: u64, request: u64, destination: AssetId<Image>, source_revision: u64 }

#[derive(Clone)]
struct CellCopy { source: (u32, u32), destination: (u32, u32) }

#[derive(Clone)]
struct Transfers { source: Handle<Image>, destination: Handle<Image>, cells: Vec<CellCopy>, resolution: u32 }

fn colour_rectangle(grid: (i32, i32), origin: (i32, i32), resolution: u32) -> (u32, u32) {
    ((grid.0 - origin.0) as u32 * resolution, (2 - (grid.1 - origin.1)) as u32 * resolution)
}
fn retained_mask(publication: &PublishedSurface, current: &[CellKey]) -> u32 {
    publication.keys.iter().filter(|k| current.contains(k)).fold(0, |mask, k| {
        mask | (1 << ((k.grid.1 - publication.origin.1) * 3 + k.grid.0 - publication.origin.0))
    })
}

struct BakeJob {
    root: Entity,
    publication: PublishedSurface,
    report: String,
    receipt: Receipt,
    copied: usize,
    baked: usize,
}

#[derive(Resource)]
pub struct SurfaceCache {
    pub generation: u64,
    signature: Option<Request>,
    context: Option<(u32, bool, u32, usize)>,
    epoch: u64,
    revision: u64,
    current: Vec<CellKey>,
    image_revisions: HashMap<AssetId<Image>, u64>,
    image_events: bevy::ecs::message::MessageCursor<AssetEvent<Image>>,
    /// Cancelled render extraction gets three frames to retire before another allocation.
    retirement: u8,
    job: Option<BakeJob>,
    ready: Option<Receipt>,
    submitted: Arc<Mutex<Option<Receipt>>>,
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
    fn ensure_context(&mut self, commands: &mut Commands, world: &WorldData, streamer: &Streamer) {
        let context = (world.world, streamer.interior.is_some(), self.resolution, std::ptr::from_ref(world.lo) as usize);
        if self.context == Some(context) { return; }
        let retire = self.job.is_some() || self.published.is_some() || self.retirement > 0;
        self.release(commands);
        self.retirement = if retire { 3 } else { 0 };
        self.context = Some(context);
    }
    pub fn coverage_for(&self, publication: &PublishedSurface) -> u32 {
        if self.context.is_none() { publication.coverage } else { retained_mask(publication, &self.current) }
    }
    pub fn with_resolution(resolution: u32) -> Self {
        Self { generation: 0, signature: None, context: None, epoch: 0, revision: 0, current: vec![], image_revisions: default(), image_events: default(), retirement: 0, job: None, ready: None, submitted: default(), published: None, resolution, rejected: false }
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
        self.epoch += 1;
        self.revision += 1;
        self.context = None;
        self.current.clear();
        self.retirement = 3;
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
    lookup_covered_status(data, origin, point, resolution, 0x1ff)
}
fn covered(slot: (i32, i32), coverage: u32) -> bool {
    (0..3).contains(&slot.0) && (0..3).contains(&slot.1) && coverage & (1 << (slot.1 * 3 + slot.0)) != 0
}
pub fn lookup_covered_status(data: &[u8], origin: (i32, i32), point: Vec2, resolution: u32, coverage: u32) -> LookupStatus {
    let cell = fnv_world::cell_of(point.x, point.y);
    let slot = (cell.0 - origin.0, cell.1 - origin.1);
    if !(0..3).contains(&slot.0) || !(0..3).contains(&slot.1) { return LookupStatus::OutOfRange; }
    let local = point / 4096.0 - Vec2::new(cell.0 as f32, cell.1 as f32);
    let square = (local * GRID as f32).floor().as_uvec2().min(UVec2::splat(GRID as u32 - 1));
    let at = height_index(slot, (square.x as usize, square.y as usize), 0, 0);
    if !covered(slot, coverage) || f32::from_le_bytes(data[at..at + 4].try_into().unwrap()) < -1e30 { return LookupStatus::InvalidHeight; }
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
        if !covered(tap_slot, coverage) || residency & (1 << (offset.1 * 3 + offset.0)) == 0 || !colour_valid(data, tap_slot, quadrant) {
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
#[derive(Component, Clone)]
struct BakeView {
    receipt: Receipt,
    height: Handle<Image>,
    resources_ready: Arc<AtomicBool>,
    transfers: Option<Transfers>,
    expected: HashSet<Entity>,
    complete: Arc<AtomicBool>,
    drawn: Arc<Mutex<HashSet<Entity>>>,
    submitted: Arc<Mutex<Option<Receipt>>>,
}

/// [design] Registered in every run, so the settings row can switch the blend while the
/// game runs, but gated: while the blend's [`MaterialGate`] is closed (never turned on, or
/// off and its materials gone, mesh_terrain_blend.rs `close_gate`) the capture material's
/// systems and the bake view extraction do not run, the capture shader is not loaded and
/// no allocator exists; the receipt and span systems run only while a bake view is in the
/// render world. Bevy's draw functions are wrapped when the first bake view appears and
/// put back once the gate closes.
pub struct SurfaceBakePlugin {
    pub gate: MaterialGate,
}
impl Plugin for SurfaceBakePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "terrain_capture.wesl");
        app.add_plugins(MaterialPlugin::<SurfaceCaptureMaterial>::gated(self.gate.clone()));
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            let (extract, closed) = (self.gate.clone(), self.gate.clone());
            render.add_systems(ExtractSchedule, extract_bake_views.run_if(move || extract.is_open()))
                .add_systems(Render, install_bake_receipts.run_if(first_bake_view).in_set(RenderSystems::Prepare))
                .add_systems(Render, uninstall_bake_receipts
                    .run_if(move |installed: Option<Res<BakeReceiptsInstalled>>| installed.is_some() && !closed.is_open())
                    .in_set(RenderSystems::Prepare))
                .add_systems(Render, reset_receipts.run_if(any_bake_view).in_set(RenderSystems::Prepare))
                .add_systems(Render, (copy_retained, submit_receipts).chain().run_if(any_bake_view).in_set(RenderSystems::Cleanup))
                .add_systems(bevy::core_pipeline::Core3d, (
                    bake_span_begin.before(bevy::core_pipeline::Core3dSystems::Prepass),
                    bake_span_end.after(bevy::core_pipeline::Core3dSystems::PostProcess),
                ).run_if(any_bake_view));
        }
    }
}

/// The bake cameras' views into the render world (what `ExtractComponentPlugin` does, but
/// gated). A view leaves with its camera: the bake camera is only ever despawned.
fn extract_bake_views(mut commands: Commands, views: Extract<Query<(RenderEntity, &BakeView)>>) {
    for (entity, view) in &views {
        commands.entity(entity).insert(view.clone());
    }
}

/// A bake view is in the render world (the receipt and span systems run only then).
fn any_bake_view(views: Query<(), With<BakeView>>) -> bool {
    !views.is_empty()
}

/// The render world's note that the receipts wrap the draw functions now.
#[derive(Resource)]
struct BakeReceiptsInstalled;

/// [design] A bake view is in the render world and the receipts are not installed yet:
/// the blend bakes for the first time since the gate opened (a run that never turns the
/// blend on keeps Bevy's own draw functions).
fn first_bake_view(views: Query<(), With<BakeView>>, installed: Option<Res<BakeReceiptsInstalled>>) -> bool {
    installed.is_none() && !views.is_empty()
}

/// Both phases' receipts, once (before the bake's passes are recorded; `DrawFunctions`
/// prepares each function when its phase renders).
fn install_bake_receipts(world: &mut World) {
    set_draw_material::<Opaque3d, (bevy::pbr::DrawMaterial, BakeReceipt)>(world);
    set_draw_material::<Transparent3d, (bevy::pbr::DrawMaterial, BakeReceipt)>(world);
    world.insert_resource(BakeReceiptsInstalled);
}

/// [design] The gate closed (no bake view, no blend or capture material left): both
/// phases get a new `DrawMaterial` command of Bevy's own, what bevy_pbr's
/// `add_render_command::<P, DrawMaterial>` installs, so an idle run draws as main does.
fn uninstall_bake_receipts(world: &mut World) {
    set_draw_material::<Opaque3d, bevy::pbr::DrawMaterial>(world);
    set_draw_material::<Transparent3d, bevy::pbr::DrawMaterial>(world);
    world.remove_resource::<BakeReceiptsInstalled>();
}

// [design] Base copies draw in Opaque3d, layer copies (alpha blended) in Transparent3d;
// while installed, both phases' DrawMaterial carries the receipt. The function keeps
// DrawMaterial's id, so prepared materials need no change either way.
fn set_draw_material<P: CachedRenderPipelinePhaseItem, C>(world: &mut World)
where
    C: RenderCommand<P> + Send + Sync + 'static,
    C::Param: bevy::ecs::system::ReadOnlySystemParam,
{
    let command = RenderCommandState::<P, C>::new(world);
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
/// and the recorder pairs a span's end with the newest span still open on the same thread
/// (bevy_render `diagnostic/internal.rs`). Each end writes its timestamp into a command
/// encoder of its own, finished and queued at that system's place in the render order
/// (`RenderContext::add_command_buffer`), never into the render context's encoder: with
/// `NVF_PLAY_TEST_PROFILE_ENCODERS=1` asking for that encoder opens an automatic
/// `enc:<system>` span first, which the begin would nest inside and the context's drop
/// would close in its place (#2661 round 8). So neither system opens an `enc:` span, every
/// pass system in between opens and closes its own, and the end closes the bake span with
/// encoder profiling on or off. Any other view, or no recorder: nothing.
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
        let mut encoder = ctx.render_device().create_command_encoder(&bevy::render::render_resource::CommandEncoderDescriptor { label: Some(BAKE_SPAN) });
        if begin {
            recorder.begin_time_span(&mut encoder, BAKE_SPAN.into());
        } else {
            recorder.end_time_span(&mut encoder);
        }
        ctx.add_command_buffer(encoder.finish());
    }
    state.apply(world);
}
fn reset_receipts(views: Query<&BakeView>, mut cameras: ResMut<bevy::render::camera::SortedCameras>) {
    // Freeze successful generations before another view pass can clear their target.
    cameras.0.retain(|camera| !views.get(camera.entity).is_ok_and(|view| view.complete.load(Ordering::Acquire)));
    for view in &views { view.drawn.lock().unwrap().clear(); }
}
fn complete_draws(expected: &HashSet<Entity>, drawn: &HashSet<Entity>) -> bool {
    !expected.is_empty() && expected == drawn
}
/// [design: bevy_render/src/lib.rs] Cleanup follows render_system, including the final
/// camera output and queue submission. This separate submission is therefore AFTER all
/// camera writes, never before a clear/upscale can erase the copied rectangles.
fn copy_retained(
    views: Query<&BakeView>,
    images: Res<bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>>,
    device: Res<bevy::render::renderer::RenderDevice>,
    queue: Res<bevy::render::renderer::RenderQueue>,
) {
    for view in &views {
        let available = images.get(&view.height).is_some() && images.get(view.receipt.destination).is_some()
            && view.transfers.as_ref().is_none_or(|p| images.get(&p.source).is_some());
        view.resources_ready.store(available, Ordering::Release);
        if !available || view.complete.load(Ordering::Acquire) || !draws_ready(&view.expected, &view.drawn.lock().unwrap()) { continue; }
        let Some(plan) = &view.transfers else { continue };
        let (Some(source), Some(destination)) = (images.get(&plan.source), images.get(&plan.destination)) else { continue };
        let mut encoder = device.create_command_encoder(&bevy::render::render_resource::CommandEncoderDescriptor { label: Some("mesh terrain retained copies") });
        for cell in &plan.cells {
            let mut src = source.texture.as_image_copy();
            src.origin.x = cell.source.0; src.origin.y = cell.source.1;
            let mut dst = destination.texture.as_image_copy();
            dst.origin.x = cell.destination.0; dst.origin.y = cell.destination.1;
            encoder.copy_texture_to_texture(src, dst, Extent3d { width: plan.resolution, height: plan.resolution, depth_or_array_layers: 1 });
        }
        queue.submit([encoder.finish()]);
        view.complete.store(true, Ordering::Release);
    }
}
fn draws_ready(expected: &HashSet<Entity>, drawn: &HashSet<Entity>) -> bool { expected == drawn }
fn submit_receipts(views: Query<&BakeView>) {
    for view in &views {
        if !view.resources_ready.load(Ordering::Acquire) { continue; }
        let complete = if view.transfers.is_some() { view.complete.load(Ordering::Acquire) }
            else { complete_draws(&view.expected, &view.drawn.lock().unwrap()) };
        if complete {
            view.complete.store(true, Ordering::Release);
            // Includes the transfer submission, even for a copy-only request.
            *view.submitted.lock().unwrap() = Some(view.receipt);
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
    captured: Query<&SurfaceCell>,
    mut inputs: Local<HashMap<Entity, u64>>,
    mut terrain: crate::terrain::TerrainMaterials,
    mut models: nvf_assets::model::ModelCtx,
    mut state: ResMut<crate::mesh_terrain_blend::BlendState>,
) {
    if streamer.interior.is_some() { return; }
    let Ok(camera) = camera.single() else { return };
    let origin = cache_origin(bevy_to_game(camera.translation()).truncate());
    inputs.retain(|e, _| captured.contains(*e));
    let stitched = stitched_dependents(&mut inputs, streamer.loaded.iter().filter_map(|(grid, entity)| {
        captured.get(*entity).ok().map(|c| (*grid, *entity, c.content_revision))
    }));
    for y in origin.1..origin.1 + 3 { for x in origin.0..origin.0 + 3 {
        let Some(&cell) = streamer.loaded.get(&(x, y)) else { continue };
        let existing = captured.get(cell).ok();
        if existing.is_some() && !stitched.contains(&(x, y)) { continue; }
        if existing.is_none() && !state.tried.insert(cell) { continue; }
        let Some(land) = world.grid_cell((x, y)).and_then(|c| c.land) else { continue };
        let tile = terrain.settings.tile;
        let Some(prep) = crate::terrain::prepare_terrain(world.lo, world.schema, land, (x, y), tile, world.neighbour_lands((x, y))) else { continue };
        let folds = terrain.folds();
        if let Some(mut surface) = crate::terrain::capture_surface(&world, prep, &mut crate::terrain::TerrainCtx { tm: &mut terrain, assets: &mut models }, folds) {
            if let Some(old) = existing { surface.content_revision = old.content_revision; }
            commands.entity(cell).try_insert(surface);
        }
    }}
}

/// [design] Epoch changes clear publications before conversion, even with no camera.
pub fn invalidate_context(mut commands: Commands, mut cache: ResMut<SurfaceCache>, world: Res<WorldData>, streamer: Res<Streamer>) {
    cache.ensure_context(&mut commands, &world, &streamer);
}

type CapturedInput = ((i32, i32), Entity, u64);
fn stitched_dependents(inputs: &mut HashMap<Entity, u64>, captures: impl Iterator<Item = CapturedInput>) -> HashSet<(i32, i32)> {
    let mut stitched = HashSet::new();
    for (grid, entity, revision) in captures {
        // First residency does not change the LAND records the neighbours already
        // stitched from WorldData. Only reconstruction of a known input dirties them.
        if inputs.insert(entity, revision).is_some_and(|previous| previous != revision) {
            for offset in [(-1, 0), (1, 0), (0, -1), (0, 1)] { stitched.insert((grid.0 + offset.0, grid.1 + offset.1)); }
        }
    }
    stitched
}

/// [design] Capture sources are retained separately from their unlit clones. Edits to
/// source materials refresh those clones and dependencies before a new request is keyed.
/// Diffuse image modifications are versioned per dependency in update_cache.
pub fn refresh_captures(
    mut cells: Query<&mut SurfaceCell>,
    folded: Res<Assets<TerrainMaterial>>,
    ordinary: Res<Assets<nvf_assets::NifMaterial>>,
    folded_events: Option<Res<Messages<AssetEvent<TerrainMaterial>>>>,
    ordinary_events: Option<Res<Messages<AssetEvent<nvf_assets::NifMaterial>>>>,
    mut cursors: Local<CaptureEvents>,
) {
    let folded_changed: HashSet<_> = folded_events.as_ref().map(|e| cursors.folded.read(e).filter_map(modified_asset).collect()).unwrap_or_default();
    let ordinary_changed: HashSet<_> = ordinary_events.as_ref().map(|e| cursors.ordinary.read(e).filter_map(modified_asset).collect()).unwrap_or_default();
    for mut cell in &mut cells {
        // Component edits/replacement also refresh the derived dependency list. Preserve
        // the producer's change tick; this bookkeeping is not another content edit.
        if cell.is_changed() { cell.bypass_change_detection().refresh_dependencies(); }
        let dirty = cell.sources.iter().any(|s| match s { CaptureSource::Folded(h) => folded_changed.contains(&h.id()), CaptureSource::Single(h, _) => ordinary_changed.contains(&h.id()) });
        if !dirty { continue; }
        let sources = cell.sources.clone();
        let mut missing = false;
        for (source, (_, draw)) in sources.iter().zip(&mut cell.quadrants) {
            *draw = match source {
                CaptureSource::Folded(h) => {
                    let Some(m) = folded.get(h) else { missing = true; continue };
                    let mut m = m.clone(); m.extension.surface_bake = true; m.extension.burn = None; m.base.fog_enabled = false;
                    CaptureDraw::Folded(Box::new(m))
                }
                CaptureSource::Single(h, layer) => {
                    let Some(m) = ordinary.get(h) else { missing = true; continue };
                    CaptureDraw::Single(SurfaceCaptureMaterial::from_standard(&m.base, *layer))
                }
            };
        }
        if missing { cell.paths = [CapturePath::HeightOnly("missing-source-material"); 4]; cell.quadrants.clear(); cell.sources.clear(); }
        cell.refresh_dependencies();
        // This is a source edit, not a LAND reconstruction; Changed catches it without
        // propagating a false stitched-LAND reconstruction to its neighbours.
    }
}
fn modified_asset<A: Asset>(event: &AssetEvent<A>) -> Option<AssetId<A>> {
    match event { AssetEvent::Modified { id } | AssetEvent::Removed { id } => Some(*id), _ => None }
}
#[derive(Default)]
pub struct CaptureEvents {
    folded: bevy::ecs::message::MessageCursor<AssetEvent<TerrainMaterial>>,
    ordinary: bevy::ecs::message::MessageCursor<AssetEvent<nvf_assets::NifMaterial>>,
}

#[allow(clippy::too_many_arguments)]
pub fn update_cache(
    mut commands: Commands,
    mut cache: ResMut<SurfaceCache>,
    world: Res<WorldData>,
    streamer: Res<Streamer>,
    camera: Query<&GlobalTransform, crate::fly::WorldCamera>,
    cells: Query<(Entity, Ref<SurfaceCell>)>,
    events: Option<Res<Messages<AssetEvent<Image>>>>,
    readiness: Option<Res<crate::mesh_terrain_blend::BlendReadiness>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
    mut captures: ResMut<Assets<SurfaceCaptureMaterial>>,
) {
    // Also done before material conversion in the production schedule. Keep the guard
    // here so direct callers cannot accept a receipt without context validation.
    cache.ensure_context(&mut commands, &world, &streamer);
    if cache.rejected || streamer.interior.is_some() { return; }
    let mut dependencies = HashSet::new();
    for (_, cell) in &cells { dependencies.extend(cell.image_dependencies().iter().copied()); }
    cache.image_revisions.retain(|id, _| dependencies.contains(id));
    if let Some(events) = events {
        let changed: Vec<_> = cache.image_events.read(&events).filter_map(|e| match e {
            AssetEvent::Modified { id } | AssetEvent::Removed { id } if dependencies.contains(id) => Some(*id), _ => None,
        }).collect();
        for id in changed { *cache.image_revisions.entry(id).or_default() += 1; }
    }
    let resolution = cache.resolution;
    let width = atlas_width(resolution);
    let Ok(camera) = camera.single() else { return };
    let eye = bevy_to_game(camera.translation());
    let origin = cache_origin(eye.truncate());
    let mut resident: Vec<_> = cells.iter().filter(|(e, c)| {
        c.world == world.world && streamer.loaded.get(&c.grid) == Some(e)
            && (origin.0..origin.0 + 3).contains(&c.grid.0)
            && (origin.1..origin.1 + 3).contains(&c.grid.1)
            && c.heights.len() == SIDE * SIDE
    }).collect();
    resident.sort_by_key(|(_, c)| c.grid);
    let keys: Vec<_> = resident.iter().map(|(entity, c)| {
        let mut images: Vec<_> = c.image_dependencies().iter().copied().map(|id| (id, cache.image_revisions.get(&id).copied().unwrap_or(0))).collect();
        images.sort_by_key(|(id, _)| *id);
        CellKey { grid: c.grid, entity: *entity, revision: c.content_revision, changed: c.last_changed().get(), images }
    }).collect();
    cache.current.clone_from(&keys);
    // Retain immutable pixels at their OLD origin, masking departing/changed cells.
    if let Some(front) = &cache.published {
        let coverage = retained_mask(front, &keys);
        if coverage != front.coverage {
            cache.revision += 1;
            let revision = cache.revision;
            let front = cache.published.as_mut().unwrap();
            front.coverage = coverage;
            front.cells = coverage.count_ones() as usize;
            front.revision = revision;
        }
    }
    let signature = Request { epoch: cache.epoch, origin, cells: keys };
    if cache.signature.as_ref() != Some(&signature) {
        if let Some(old) = cache.job.take() {
            commands.entity(old.root).despawn();
            cache.retirement = 3;
        }
        cache.signature = None;
        cache.ready = None;
        *cache.submitted.lock().unwrap() = None;
    }
    // Consume only exact receipts for the current request and source publication.
    let publish = cache.ready.take();
    let received = cache.submitted.lock().unwrap().take();
    cache.ready = received;
    if cache.signature.as_ref() == Some(&signature) {
        if cache.job.as_ref().is_some_and(|j| Some(j.receipt) == publish
            && j.receipt.epoch == cache.epoch && j.receipt.request == cache.generation
            && j.receipt.destination == j.publication.colour.id()
            && j.receipt.source_revision == cache.published.as_ref().map_or(0, |p| p.revision)) {
            let job = cache.job.take().unwrap();
            commands.entity(job.root).despawn();
            cache.revision += 1;
            let mut publication = job.publication;
            publication.revision = cache.revision;
            info!("{}", job.report);
            cache.published = Some(publication);
        }
        return;
    }
    if cache.retirement > 0 { cache.retirement -= 1; return; }
    if resident.is_empty() { cache.signature = Some(signature); return; }
    // A pure removal changes only enabled coverage; no colour allocation or transfer.
    if cache.published.as_ref().is_some_and(|p| signature.cells.iter().all(|k| p.keys.contains(k))) {
        cache.signature = Some(signature);
        return;
    }
    // One fresh back allocation only after the previous publication's materials have
    // switched. Combined with cancellation retirement this bounds rapid replanning.
    if cache.published.as_ref().is_some_and(|p| readiness.as_ref().is_some_and(|r| !r.settled(p.revision))) { return; }
    cache.generation += 1;
    cache.signature = Some(signature.clone());
    let copied: HashSet<_> = cache.published.as_ref().map(|p| resident.iter().filter(|(_, c)| c.colour_mask() != 0 && signature.cells.iter().any(|k| k.grid == c.grid && p.keys.contains(k))).map(|(_, c)| c.grid).collect()).unwrap_or_default();

    let mut colour = Image::new_fill(
        Extent3d { width, height: width, depth_or_array_layers: 1 },
        TextureDimension::D2, &[0; 8], TextureFormat::Rgba16Float,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    colour.texture_descriptor.usage |= TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC | TextureUsages::COPY_DST;
    colour.sampler = ImageSampler::linear();
    let colour = images.add(colour);
    let mut heights = empty_height_atlas();
    let mut expected = HashSet::new();
    let root = commands.spawn((Name::new("Mesh terrain surface bake"), game_to_bevy(), Visibility::Inherited)).id();
    let slots = resident.iter().map(|(_, c)| (c.grid.0 - origin.0, c.grid.1 - origin.1)).collect();
    for (_, cell) in &resident {
        let slot = (cell.grid.0 - origin.0, cell.grid.1 - origin.1);
        encode_surface_cell(&mut heights, slot, cell, &slots);
        if copied.contains(&cell.grid) { continue; }
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
    let source_revision = cache.published.as_ref().map_or(0, |p| p.revision);
    let receipt = Receipt { epoch: cache.epoch, request: cache.generation, destination: colour.id(), source_revision };
    let transfers = cache.published.as_ref().filter(|_| !copied.is_empty()).map(|p| Transfers {
        source: p.colour.clone(), destination: colour.clone(), resolution,
        cells: copied.iter().map(|&grid| CellCopy { source: colour_rectangle(grid, p.origin, resolution), destination: colour_rectangle(grid, origin, resolution) }).collect(),
    });
    let baked = resident.iter().filter(|(_, c)| !copied.contains(&c.grid) && !c.quadrants.is_empty()).count();
    info!("mesh terrain blend: assembly epoch={} request={} copied={} baked={} retained-origin={:?} destination-origin={origin:?}", cache.epoch, cache.generation, copied.len(), baked, cache.published.as_ref().map(|p| p.origin));
    if expected.is_empty() && transfers.is_none() {
        cache.ready = Some(receipt);
    } else { commands.spawn((
        Camera3d::default(), bevy::render::view::NoIndirectDrawing,
        BakeView { receipt, height: height.clone(), resources_ready: default(), transfers, expected, complete: default(), drawn: default(), submitted: cache.submitted.clone() },
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
        root, receipt, copied: copied.len(), baked,
        publication: PublishedSurface { colour, height, origin, cells: resident.len(), resolution, revision: 0,
            coverage: signature.cells.iter().fold(0, |m, k| m | (1 << ((k.grid.1-origin.1)*3+k.grid.0-origin.0))), keys: signature.cells },
        report,
    });
}

/// [design] Test aid only. Counts every main frame after an initially prepared retained
/// witness exists. It checks coherent CPU bytes and prepared material acknowledgements;
/// screenshots remain necessary to judge the actual rendered contact.
#[derive(Resource)]
struct ContinuityProbe {
    start: f64, duration: f64, done: bool, armed: bool,
    witnesses: Vec<(Entity, CellKey, Vec2)>,
    camera: Option<(i32, i32)>, colour: Option<AssetId<Image>>, request: u64,
    frames: u64, transitions: u64, pending: u64, retained: u64, pending_retained: u64, unwitnessed_pending: u64, replacements: u64,
    unpublished: u64, missing_enabled: u64, missing_prepared: u64, failures: u64,
    copied: usize, baked: usize,
}
pub fn install_continuity_probe(app: &mut App) {
    let Ok(value) = std::env::var("NVF_PLAY_TEST_MESH_BLEND_CONTINUITY") else { return };
    let Some((start, duration)) = value.split_once(':').and_then(|(a, b)| Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?))) else {
        warn!("mesh terrain blend: continuity rejected aid; expected start-seconds:duration-seconds"); return;
    };
    if !start.is_finite() || !duration.is_finite() || start < 0.0 || !(1.0..=120.0).contains(&duration) { return; }
    app.insert_resource(ContinuityProbe {
        start, duration, done: false, armed: false, witnesses: vec![], camera: None, colour: None, request: 0,
        frames: 0, transitions: 0, pending: 0, retained: 0, pending_retained: 0, unwitnessed_pending: 0, replacements: 0,
        unpublished: 0, missing_enabled: 0, missing_prepared: 0, failures: 0, copied: 0, baked: 0,
    }).add_systems(PostUpdate, continuity_probe.after(crate::mesh_terrain_blend::BlendConversion).after(update_cache).run_if(crate::mesh_terrain_blend::active).run_if(resource_exists::<nvf_play::pause::PlayClock>));
}
type ProbeShapes<'w, 's> = Query<'w, 's, (Entity, &'static nvf_assets::nif_surface_blend::BlendShape, &'static GlobalTransform, Option<&'static MeshMaterial3d<nvf_assets::nif_surface_blend::NifBlendMaterial>>)>;
#[allow(clippy::too_many_arguments)]
fn continuity_probe(
    mut probe: ResMut<ContinuityProbe>, time: nvf_play::pause::AidTime, cache: Res<SurfaceCache>,
    camera: Query<&GlobalTransform, crate::fly::WorldCamera>, shapes: ProbeShapes,
    images: Res<Assets<Image>>, blends: Res<Assets<nvf_assets::nif_surface_blend::NifBlendMaterial>>,
    ready: Res<crate::mesh_terrain_blend::BlendReadiness>,
) {
    let Some(now) = time.world_secs().map(f64::from) else { return };
    if probe.done || now < probe.start { return; }
    if now >= probe.start + probe.duration || probe.frames >= 20000 {
        probe.done = true;
        let settled = cache.job.is_none() && cache.published.as_ref().is_some_and(|p| ready.settled(p.revision));
        info!("mesh terrain blend: continuity summary frames={} crossings={} pending-frames={} retained-lookups={} pending-retained-lookups={} unwitnessed-pending-frames={} replacements={} published-false={} missing-enabled={} missing-prepared={} failures={} copied={} baked={} settled={settled}", probe.frames, probe.transitions, probe.pending, probe.retained, probe.pending_retained, probe.unwitnessed_pending, probe.replacements, probe.unpublished, probe.missing_enabled, probe.missing_prepared, probe.failures, probe.copied, probe.baked);
        return;
    }
    let camera_cell = camera.single().ok().map(|c| { let p = bevy_to_game(c.translation()); fnv_world::cell_of(p.x, p.y) });
    if !probe.armed {
        let (Some(front), Some(centre)) = (&cache.published, camera_cell) else { return };
        for (entity, shape, transform, drawn) in &shapes {
            let Some(enabled) = &shape.enabled else { continue };
            if !ready.usable(enabled.id()) || drawn.is_none() { continue; }
            let point = bevy_to_game(transform.translation()).truncate();
            let grid = fnv_world::cell_of(point.x, point.y);
            if grid != centre { continue; }
            let Some(key) = cache.current.iter().find(|k| k.grid == grid) else { continue };
            let Some(data) = images.get(&front.height).and_then(|i| i.data.as_ref()) else { continue };
            if lookup_covered_status(data, front.origin, point, front.resolution, front.coverage) != LookupStatus::Success { continue; }
            probe.witnesses.push((entity, key.clone(), point));
            if probe.witnesses.len() == 32 { break; }
        }
        if probe.witnesses.is_empty() { return; }
        probe.armed = true; probe.camera = camera_cell; probe.colour = Some(front.colour.id());
        info!("mesh terrain blend: continuity armed witnesses={}", probe.witnesses.len());
    }
    probe.frames += 1;
    let transition = matches!((camera_cell, probe.camera), (Some(a), Some(b)) if a != b);
    probe.transitions += u64::from(transition);
    probe.camera = camera_cell;
    let colour = cache.published.as_ref().map(|p| p.colour.id());
    if colour.is_some() && colour != probe.colour { probe.replacements += 1; probe.colour = colour; }
    let pending = cache.job.is_some() || cache.retirement > 0 || cache.published.as_ref().is_some_and(|p| !ready.settled(p.revision));
    probe.pending += u64::from(pending);
    probe.unpublished += u64::from(cache.published.is_none());
    if cache.generation != probe.request {
        probe.request = cache.generation;
        if let Some(job) = &cache.job {
            probe.copied += job.copied; probe.baked += job.baked;
        }
    }
    let mut retained = 0;
    let mut missing_enabled = 0;
    let mut missing_prepared = 0;
    let mut failures = u64::from(camera_cell.is_none());
    for (entity, key, point) in &probe.witnesses {
        if !cache.current.contains(key) { continue; }
        retained += 1;
        let Ok((_, shape, _, drawn)) = shapes.get(*entity) else { missing_enabled += 1; continue };
        let Some(enabled) = shape.enabled.as_ref().filter(|h| drawn.is_some_and(|d| &d.0 == *h)) else { missing_enabled += 1; continue };
        if !ready.usable(enabled.id()) { missing_prepared += 1; }
        let Some(material) = blends.get(enabled) else { failures += 1; continue };
        let e = &material.extension;
        let Some(data) = e.mesh_terrain_height.as_ref().and_then(|h| images.get(h)).and_then(|i| i.data.as_ref()) else { failures += 1; continue };
        let origin = (e.mesh_terrain_origin.x as i32, e.mesh_terrain_origin.y as i32);
        let resolution = e.mesh_terrain_colour.as_ref().and_then(|h| images.get(h)).map_or(0, |i| i.texture_descriptor.size.width / 3);
        if resolution == 0 || lookup_covered_status(data, origin, *point, resolution, e.mesh_terrain_origin.z as u32) != LookupStatus::Success { failures += 1; }
    }
    probe.retained += retained;
    if pending { probe.pending_retained += retained; probe.unwitnessed_pending += u64::from(retained == 0); }
    probe.missing_enabled += missing_enabled;
    probe.missing_prepared += missing_prepared;
    probe.failures += failures;
    info!("mesh terrain blend: continuity frame={} camera-cell={camera_cell:?} transition={transition} pending={pending} published={} retained={retained} missing-enabled={missing_enabled} missing-prepared={missing_prepared} failures={failures} copied={} baked={}", probe.frames, cache.published.is_some(), probe.copied, probe.baked);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn surface(paths: [CapturePath; 4]) -> SurfaceCell {
        SurfaceCell { world: 0, grid: (-17, 0), heights: vec![42.5; SIDE * SIDE], quadrants: vec![], paths, content_revision: 0, sources: vec![], dependencies: default() }
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
            receipt: Receipt { epoch: 1, request: 7, destination: Handle::<Image>::default().id(), source_revision: 0 }, height: default(), resources_ready: Arc::new(AtomicBool::new(true)), transfers: None, expected: HashSet::from([a, b]), complete: default(),
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
        assert_eq!(submitted.lock().unwrap().as_ref().map(|r| r.request), Some(7));
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
        let cell = app.world_mut().spawn(SurfaceCell { world: 0, grid: (0, 0), heights: vec![0.0; SIDE * SIDE], quadrants: vec![], paths: [CapturePath::Fallback { layers: 0 }; 4], content_revision: 0, sources: vec![], dependencies: default() }).id();
        app.insert_resource(Streamer {
            radius: 1, loaded: [((0, 0), cell)].into(), fades: default(), fade_secs: 0.0, settled: true,
            arrival: None, per_frame: 1, spawning: default(), dying: default(), ahead: default(),
            hide: default(), interior: None, light_scale: 1.0, managed: default(), manage_actors: false,
            stats: default(), buffer: default(), centre: None, waiting: default(), model_overrides: default(),
        });
        app.world_mut().spawn((Camera3d::default(), crate::fly::FlyCam { yaw: 0.0, pitch: 0.0, speed: 0.0 }, GlobalTransform::from_translation(Vec3::new(2048.0, 600.0, -2048.0))));
        for _ in 0..3 { app.world_mut().run_system_once(update_cache).unwrap(); }
        let cache = app.world().resource::<SurfaceCache>();
        assert!(cache.job.is_none() && cache.published.is_none() && cache.generation == 1);
        assert!(app.world().resource::<Assets<Image>>().is_empty());
    }
    fn cache_fixture() -> (App, impl Sized) {
        let (mut app, files) = crate::outfits::tests::asset_fixture();
        app.insert_resource(crate::outfits::tests::empty_world_data())
            .insert_resource(SurfaceCache::with_resolution(512)).init_resource::<Assets<TerrainMaterial>>()
            .init_resource::<Assets<SurfaceCaptureMaterial>>()
            .init_resource::<Messages<AssetEvent<Image>>>();
        app.insert_resource(Streamer {
            radius: 2, loaded: default(), fades: default(), fade_secs: 0.0, settled: true,
            arrival: None, per_frame: 1, spawning: default(), dying: default(), ahead: default(),
            hide: default(), interior: None, light_scale: 1.0, managed: default(), manage_actors: false,
            stats: default(), buffer: default(), centre: None, waiting: default(), model_overrides: default(),
        });
        for y in -3..=3 { for x in -3..=3 {
            let diffuse = app.world_mut().resource_mut::<Assets<Image>>().add(Image::default());
            let mut cell = surface([CapturePath::Fallback { layers: 0 }; 4]);
            cell.grid = (x, y);
            cell.quadrants.push((Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, RenderAssetUsages::default()),
                CaptureDraw::Single(SurfaceCaptureMaterial::from_standard(&StandardMaterial { base_color_texture: Some(diffuse), ..default() }, false))));
            cell.refresh_dependencies();
            let entity = app.world_mut().spawn(cell).id();
            app.world_mut().resource_mut::<Streamer>().loaded.insert((x, y), entity);
        }}
        app.world_mut().spawn((Camera3d::default(), crate::fly::FlyCam { yaw: 0.0, pitch: 0.0, speed: 0.0 }, GlobalTransform::from_translation(Vec3::new(1024.0, 600.0, -1024.0))));
        tick(&mut app);
        finish(&mut app);
        (app, files)
    }
    fn tick(app: &mut App) {
        use bevy::ecs::system::RunSystemOnce;
        app.world_mut().run_system_once(update_cache).unwrap();
    }
    fn finish(app: &mut App) {
        let cache = app.world_mut().resource_mut::<SurfaceCache>();
        *cache.submitted.lock().unwrap() = Some(cache.job.as_ref().unwrap().receipt);
        tick(app);
        assert!(app.world().resource::<SurfaceCache>().job.is_some(), "extra main-frame publication delay");
        tick(app);
        assert!(app.world().resource::<SurfaceCache>().job.is_none());
    }
    fn move_camera(app: &mut App, cell: (i32, i32)) {
        let mut q = app.world_mut().query_filtered::<&mut GlobalTransform, With<crate::fly::FlyCam>>();
        *q.single_mut(app.world_mut()).unwrap() = GlobalTransform::from_translation(Vec3::new(cell.0 as f32 * 4096.0 + 1024.0, 600.0, -(cell.1 as f32 * 4096.0 + 1024.0)));
    }
    fn work_counts(app: &mut App) -> (usize, usize) {
        let view = app.world_mut().query::<&BakeView>().single(app.world()).unwrap();
        (view.transfers.as_ref().map_or(0, |p| p.cells.len()), view.expected.len())
    }
    type CellSet = HashSet<(i32, i32)>;
    fn work_sets(app: &mut App) -> (CellSet, CellSet) {
        let origin = app.world().resource::<SurfaceCache>().job.as_ref().unwrap().publication.origin;
        let view = app.world_mut().query::<&BakeView>().single(app.world()).unwrap();
        let resolution = app.world().resource::<SurfaceCache>().resolution;
        let copied = view.transfers.as_ref().map(|p| p.cells.iter().map(|c| (origin.0 + (c.destination.0 / resolution) as i32, origin.1 + 2 - (c.destination.1 / resolution) as i32)).collect()).unwrap_or_default();
        let baked = view.expected.iter().map(|entity| {
            let t = app.world().get::<Transform>(*entity).unwrap().translation;
            (origin.0 + (t.x / 4096.0) as i32, origin.1 + (t.y / 4096.0) as i32)
        }).collect();
        (copied, baked)
    }
    fn region(centre: (i32, i32)) -> CellSet {
        (-1..=1).flat_map(|y| (-1..=1).map(move |x| (centre.0 + x, centre.1 + y))).collect()
    }
    #[test]
    fn axial_diagonal_negative_and_backtracking_copy_exact_sets_keep_old_origin() {
        for (destination, expected) in [((1, 0), (6, 3)), ((1, 1), (4, 5)), ((-1, -1), (4, 5)), ((-1, 0), (6, 3))] {
            let (mut app, _files) = cache_fixture();
            let front = app.world().resource::<SurfaceCache>().published.clone().unwrap();
            move_camera(&mut app, destination);
            tick(&mut app);
            assert_eq!(work_counts(&mut app), expected);
            let old_region = region((0, 0));
            let new_region = region(destination);
            assert_eq!(work_sets(&mut app), (old_region.intersection(&new_region).copied().collect(), new_region.difference(&old_region).copied().collect()));
            for _ in 0..5 {
                tick(&mut app);
                let cache = app.world().resource::<SurfaceCache>();
                let retained = cache.published.as_ref().unwrap();
                assert_eq!((retained.colour.id(), retained.origin), (front.colour.id(), front.origin));
                let height = app.world().resource::<Assets<Image>>().get(&retained.height).unwrap().data.as_ref().unwrap();
                assert_eq!(lookup_covered_status(height, retained.origin, Vec2::splat(1024.0), 1024, retained.coverage), LookupStatus::Success);
            }
            finish(&mut app);
            assert_ne!(app.world().resource::<SurfaceCache>().published.as_ref().unwrap().colour.id(), front.colour.id());
            move_camera(&mut app, (0, 0)); tick(&mut app);
            assert_eq!(work_counts(&mut app), expected);
            finish(&mut app);
        }
    }
    #[test]
    fn rapid_crossings_reject_cancelled_receipts_and_bound_allocations() {
        let (mut app, _files) = cache_fixture();
        let colour = app.world().resource::<SurfaceCache>().published.as_ref().unwrap().colour.id();
        move_camera(&mut app, (1, 0)); tick(&mut app);
        let stale = app.world().resource::<SurfaceCache>().job.as_ref().unwrap().receipt;
        move_camera(&mut app, (1, 1)); tick(&mut app);
        assert!(app.world().resource::<SurfaceCache>().job.is_none());
        *app.world().resource::<SurfaceCache>().submitted.lock().unwrap() = Some(stale);
        for _ in 0..4 {
            tick(&mut app);
            assert_eq!(app.world().resource::<SurfaceCache>().published.as_ref().unwrap().colour.id(), colour);
        }
        let receipt = app.world().resource::<SurfaceCache>().job.as_ref().unwrap().receipt;
        assert_ne!(receipt, stale);
        // Stale and out-of-order receipts never satisfy the new job.
        *app.world().resource::<SurfaceCache>().submitted.lock().unwrap() = Some(stale);
        tick(&mut app); tick(&mut app);
        assert_eq!(app.world().resource::<SurfaceCache>().job.as_ref().unwrap().receipt, receipt);
        finish(&mut app);
    }
    #[test]
    fn unchanged_frames_and_residency_removal_do_no_colour_work() {
        let (mut app, _files) = cache_fixture();
        let before = app.world().resource::<Assets<Image>>().len();
        let generation = app.world().resource::<SurfaceCache>().generation;
        for _ in 0..4 { tick(&mut app); }
        app.world_mut().resource_mut::<Streamer>().loaded.remove(&(1, 1));
        tick(&mut app);
        let cache = app.world().resource::<SurfaceCache>();
        assert_eq!(cache.generation, generation);
        assert!(cache.job.is_none());
        assert_eq!(cache.published.as_ref().unwrap().coverage.count_ones(), 8);
        assert_eq!(app.world().resource::<Assets<Image>>().len(), before);
        // A surviving SurfaceCell of the removed entity must not restore residency.
        tick(&mut app);
        assert_eq!(app.world().resource::<SurfaceCache>().published.as_ref().unwrap().cells, 8);
    }
    #[test]
    fn capture_and_dependent_image_changes_bake_only_one_cell() {
        for image_change in [false, true] {
            let (mut app, _files) = cache_fixture();
            let entity = app.world().resource::<Streamer>().loaded[&(0, 0)];
            if image_change {
                let dependency = *app.world().get::<SurfaceCell>(entity).unwrap().image_dependencies().iter().next().unwrap();
                app.world_mut().resource_mut::<Messages<AssetEvent<Image>>>().write(AssetEvent::Modified { id: dependency });
            } else {
                app.world_mut().get_mut::<SurfaceCell>(entity).unwrap().heights[0] = 123.0;
            }
            tick(&mut app);
            assert_eq!(work_counts(&mut app), (8, 1));
            let mut unchanged = region((0, 0)); unchanged.remove(&(0, 0));
            assert_eq!(work_sets(&mut app), (unchanged, HashSet::from([(0, 0)])));
            assert_eq!(app.world().resource::<SurfaceCache>().published.as_ref().unwrap().coverage.count_ones(), 8);
            finish(&mut app);
        }
    }
    #[test]
    fn coverage_rejects_unloaded_diagonal_footprints_and_invalid_quadrants() {
        let mut data = empty_height_atlas();
        let cell = surface([CapturePath::Fallback { layers: 0 }; 4]);
        let slots = HashSet::from([(0, 0), (1, 0), (0, 1), (1, 1)]);
        for &slot in &slots { encode_surface_cell(&mut data, slot, &cell, &slots); }
        let full = 0x1ff;
        assert_eq!(lookup_covered_status(&data, (-2, -2), Vec2::splat(-4096.1), 1024, full), LookupStatus::Success);
        let without_diagonal = full & !(1 << 4);
        assert_eq!(lookup_covered_status(&data, (-2, -2), Vec2::splat(-4096.1), 1024, without_diagonal), LookupStatus::RejectedFootprint);
        assert_eq!(lookup_covered_status(&data, (-2, -2), Vec2::splat(-3072.0), 1024, without_diagonal), LookupStatus::InvalidHeight);
        encode_colour_mask(&mut data, (0, 0), 0b0111);
        assert_eq!(lookup_covered_status(&data, (-2, -2), Vec2::splat(-4096.1), 1024, full), LookupStatus::InvalidColourQuadrant);
    }
    #[test]
    fn epoch_invalidates_before_receipts_even_without_camera_and_off_on() {
        let (mut app, _files) = cache_fixture();
        move_camera(&mut app, (1, 0)); tick(&mut app);
        let stale = app.world().resource::<SurfaceCache>().job.as_ref().unwrap().receipt;
        *app.world().resource::<SurfaceCache>().submitted.lock().unwrap() = Some(stale);
        let camera = app.world_mut().query_filtered::<Entity, With<crate::fly::FlyCam>>().single(app.world()).unwrap();
        app.world_mut().despawn(camera);
        app.world_mut().resource_mut::<WorldData>().world = 42;
        tick(&mut app);
        let cache = app.world().resource::<SurfaceCache>();
        assert!(cache.published.is_none() && cache.job.is_none() && cache.ready.is_none());
        assert_ne!(cache.epoch, stale.epoch);
        app.world_mut().resource_mut::<WorldData>().world = 0;
        tick(&mut app);
        assert!(app.world().resource::<SurfaceCache>().published.is_none());
    }
    #[test]
    fn interior_and_incompatible_resolution_cancel_pending_work_without_a_camera() {
        for interior in [false, true] {
            let (mut app, _files) = cache_fixture();
            move_camera(&mut app, (1, 0)); tick(&mut app);
            let stale = app.world().resource::<SurfaceCache>().job.as_ref().unwrap().receipt;
            *app.world().resource::<SurfaceCache>().submitted.lock().unwrap() = Some(stale);
            let camera = app.world_mut().query_filtered::<Entity, With<crate::fly::FlyCam>>().single(app.world()).unwrap();
            app.world_mut().despawn(camera);
            if interior { app.world_mut().resource_mut::<Streamer>().interior = Some(42); }
            else { app.world_mut().resource_mut::<SurfaceCache>().resolution = 2048; }
            tick(&mut app);
            let cache = app.world().resource::<SurfaceCache>();
            assert!(cache.published.is_none() && cache.job.is_none() && cache.ready.is_none());
            assert_ne!(cache.epoch, stale.epoch);
        }
    }

    #[test]
    fn copy_only_receipt_waits_for_transfer_resources_and_completion() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        let shared = Arc::new(Mutex::new(None));
        let view = BakeView {
            receipt: Receipt { epoch: 3, request: 4, destination: Handle::<Image>::default().id(), source_revision: 2 },
            height: default(), resources_ready: default(),
            expected: default(), drawn: default(), complete: default(), submitted: shared.clone(),
            transfers: Some(Transfers { source: default(), destination: default(), cells: vec![CellCopy { source: (0, 0), destination: (1024, 2048) }], resolution: 1024 }),
        };
        let completed = view.complete.clone();
        let resources = view.resources_ready.clone();
        app.world_mut().spawn(view);
        app.world_mut().run_system_once(submit_receipts).unwrap();
        assert!(shared.lock().unwrap().is_none(), "no receipt when resources/copies are still missing");
        completed.store(true, Ordering::Release);
        app.world_mut().run_system_once(submit_receipts).unwrap();
        assert!(shared.lock().unwrap().is_none(), "a completed copy still requires height and colour GPU resources");
        resources.store(true, Ordering::Release);
        app.world_mut().run_system_once(submit_receipts).unwrap();
        assert_eq!(shared.lock().unwrap().as_ref().map(|r| (r.epoch, r.request)), Some((3, 4)));
        assert_eq!(colour_rectangle((-2, -3), (-3, -3), 1024), (1024, 2048));
        assert_eq!(colour_rectangle((-3, -1), (-3, -3), 1024), (0, 0));
    }
    #[test]
    fn stitched_inputs_ignore_first_residency_and_dirty_cardinals_on_reconstruction_only() {
        let mut world = World::new();
        let a = world.spawn_empty().id();
        let b = world.spawn_empty().id();
        let mut inputs = HashMap::new();
        let capture = ((-17, -2), a, 1);
        assert!(stitched_dependents(&mut inputs, [capture].into_iter()).is_empty());
        // An entering neighbour already supplied its LAND layers through WorldData.
        let entering = ((-16, -2), b, 2);
        assert!(stitched_dependents(&mut inputs, [capture, entering].into_iter()).is_empty());
        assert_eq!(stitched_dependents(&mut inputs, [((-17, -2), a, 3), entering].into_iter()), HashSet::from([(-18, -2), (-16, -2), (-17, -3), (-17, -1)]));
        assert!(stitched_dependents(&mut inputs, [((-17, -2), a, 3), entering].into_iter()).is_empty(), "stitched refresh keeps the primary revision and cannot propagate recursively");
    }
    #[test]
    fn source_material_edits_refresh_cloned_capture_inputs_and_dependencies() {
        use bevy::ecs::system::RunSystemOnce;
        let (mut app, _files) = crate::outfits::tests::asset_fixture();
        app.init_resource::<Assets<TerrainMaterial>>().init_resource::<Messages<AssetEvent<nvf_assets::NifMaterial>>>();
        let source = app.world_mut().resource_mut::<Assets<nvf_assets::NifMaterial>>().add(nvf_assets::NifMaterial::default());
        let image = app.world_mut().resource_mut::<Assets<Image>>().add(Image::default());
        let mut cell = surface([CapturePath::Fallback { layers: 0 }; 4]);
        cell.sources.push(CaptureSource::Single(source.clone(), false));
        cell.quadrants.push((Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, RenderAssetUsages::default()), CaptureDraw::Single(SurfaceCaptureMaterial::from_standard(&StandardMaterial::default(), false))));
        let entity = app.world_mut().spawn(cell).id();
        app.world_mut().resource_mut::<Assets<nvf_assets::NifMaterial>>().get_mut(&source).unwrap().base.base_color_texture = Some(image.clone());
        app.world_mut().resource_mut::<Messages<AssetEvent<nvf_assets::NifMaterial>>>().write(AssetEvent::Modified { id: source.id() });
        app.world_mut().run_system_once(refresh_captures).unwrap();
        let cell = app.world().get::<SurfaceCell>(entity).unwrap();
        assert_eq!(cell.image_dependencies(), &HashSet::from([image.id()]));
        let CaptureDraw::Single(draw) = &cell.quadrants[0].1 else { panic!() };
        assert_eq!(draw.diffuse, Some(image));
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
        let extra = world.spawn_empty().id();
        assert!(!complete_draws(&expected, &HashSet::from([a, b, extra])), "unplanned draws can contaminate the destination");
    }
}
