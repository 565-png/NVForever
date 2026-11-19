//! Default-off field for authored PBR material sets (#2981; docs/spec-notes/pbr-materials.md
//! "Where a set does not apply, and checking one"), behind `NVF_PLAY_TEST_PBR_FIELD=1`, modelled on
//! `bc5_field.rs`. Every file is generated here, into a fresh folder under
//! `NVFOREVER_HOME`, and read through a real VFS over it; the panels' materials come from the
//! real `shape_material` with the device's actual BC support; two GPU readbacks are judged.
//!
//! Four panels, side by side under one directional light and a fixed ambient light, all with
//! the same diffuse bytes under four names:
//! - `plain`: no set;
//! - `authored`: `_orm` (occlusion 40/255, roughness 1, metallic 0; 16x16) and a two-channel
//!   `_n` (unsigned BC5, 32x32, X 0 and Y 0.6: not flat), the `_n` decoded on a worker thread
//!   (`texture::worker_texture_exact`) and adopted by the main-thread build;
//! - `reference`: the same `_orm`, and the same normal as a three-channel map in the game's
//!   normal slot;
//! - `flat`: the same `_orm`, no normal map.
//!
//! With the run's setting on, `authored` must be darker than `plain` (the occlusion), match
//! `reference` (the two-channel rebuild gives the three-channel normal) and `flat` must
//! differ from `reference` (the normal is visible at all). Off, `plain`, `authored` and
//! `flat` must agree. The comparator is an engineering policy, not a fact about the game.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bevy::asset::RenderAssetUsages;
use bevy::camera::{Exposure, RenderTarget, ScalingMode, visibility::RenderLayers};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::gpu_readback::{ReadbackComplete, ReadbackOnce};
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::view::ViewTarget;

use crate::{GameData, NifMaterial, TextureCache};
use crate::nif_material::TransmissionAnchor;
use crate::nif_spawn::{Props, shape_material};
use crate::texture::{worker_texture_exact, TextureSnapshot, TextureUsage};

const LAYER: usize = 30;
const PANEL: usize = 128;
const PANELS: usize = 4;
const WIDTH: usize = PANEL * PANELS;
const HEIGHT: usize = PANEL;
/// The judged region of each panel: its middle half, away from the edges.
const ROI: std::ops::Range<usize> = 32..96;
const NAMES: [&str; PANELS] = ["plain", "authored", "reference", "flat"];

/// The diffuse (sRGB grey 160), the ORM (occlusion 40/255, roughness 1, metallic 0) and the
/// normal (X 0, Y 0.6, Z 0.8) as generated texels, RGBA byte order.
const DIFFUSE: [u8; 4] = [160, 160, 160, 255];
const ORM: [u8; 4] = [40, 255, 0, 255];
const NORMAL_XY: [u8; 2] = [128, 204];
const NORMAL_RGB: [u8; 4] = [128, 204, 230, 255];
/// Sides of the generated maps: each its own (the format's rule, pbr-materials.md).
const DIFFUSE_SIDE: u32 = 8;
const ORM_SIDE: u32 = 16;
const NORMAL_SIDE: u32 = 32;
const N_PATH: &str = "textures\\pbr\\field\\authored_n.dds";
const PBR_FILES: [&str; 4] = ["textures\\pbr\\field\\authored_orm.dds", N_PATH, "textures\\pbr\\field\\reference_orm.dds", "textures\\pbr\\field\\flat_orm.dds"];

/// The comparator [engineering policy]: the occlusion darkens `authored` by at least
/// `MIN_OCCLUSION` (mean luminance, bytes), `authored` and `reference` agree within
/// `SAME`, `reference` and `flat` differ by at least `MIN_NORMAL`; off, the plain-material
/// panels agree within `SAME_OFF`. A judged region's mean must lie inside `LIT` and no texel
/// in it may be clipped at `CLIP`.
const MIN_OCCLUSION: f64 = 20.0;
const SAME: f64 = 3.0;
const MIN_NORMAL: f64 = 5.0;
const SAME_OFF: f64 = 2.0;
const LIT: std::ops::Range<f64> = 8.0..247.0;
const CLIP: u8 = 254;

#[derive(Component)]
struct FieldOwned;

#[derive(Component, Clone, ExtractComponent)]
#[extract_app(RenderApp)]
struct FieldCamera;

#[derive(Resource, Clone, Default)]
struct RenderFrames(Arc<AtomicU64>);

#[derive(Resource)]
struct Field {
    started: Option<Instant>,
    output: Option<PathBuf>,
    scene: Option<Scene>,
    on: bool,
    pending: Option<Entity>,
    completed: u8,
    after_frame: u64,
    done: bool,
}

struct Scene {
    entities: Vec<Entity>,
    target: Handle<Image>,
}

pub(crate) fn install(app: &mut App) {
    let Some(value) = std::env::var_os("NVF_PLAY_TEST_PBR_FIELD") else { return };
    if value != "1" {
        error!("pbr field: FAIL expected NVF_PLAY_TEST_PBR_FIELD=1");
        return;
    }
    let frames = RenderFrames::default();
    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        error!("pbr field: FAIL missing normal RenderApp");
        return;
    };
    render.insert_resource(frames.clone()).add_systems(Render, count_rendered.in_set(RenderSystems::Cleanup));
    app.add_plugins(ExtractComponentPlugin::<FieldCamera>::default())
        .insert_resource(frames)
        .insert_resource(Field { started: None, output: None, scene: None, on: false, pending: None, completed: 0, after_frame: 0, done: false })
        .add_systems(Update, tick);
}

fn count_rendered(views: Query<(&ExtractedCamera, &ViewTarget), With<FieldCamera>>, frames: Res<RenderFrames>) {
    if let Ok((camera, _)) = views.single() {
        if camera.physical_target_size == Some(UVec2::new(WIDTH as u32, HEIGHT as u32)) {
            frames.0.fetch_add(1, Ordering::AcqRel);
        }
    }
}

fn tick(world: &mut World) {
    let Some(mut field) = world.remove_resource::<Field>() else { return };
    let started = *field.started.get_or_insert_with(Instant::now);
    if !field.done {
        if let Err(reason) = advance(world, &mut field, started.elapsed()) {
            error!("pbr field: FAIL {reason}");
            field.done = true;
        }
    }
    if field.done {
        if let Some(entity) = field.pending.take() {
            world.despawn(entity);
        }
        if let Some(scene) = field.scene.take() {
            for entity in scene.entities {
                world.despawn(entity);
            }
        }
    }
    world.insert_resource(field);
}

fn advance(world: &mut World, field: &mut Field, elapsed: Duration) -> Result<(), String> {
    if elapsed >= Duration::from_secs(30) {
        return Err("timeout before two completed rendered comparisons".into());
    }
    check_layer(world)?;
    if field.scene.is_none() {
        let Some(data) = world.get_resource::<GameData>().cloned() else { return Ok(()) };
        let home = std::env::var_os("NVFOREVER_HOME").ok_or("missing explicit owned NVFOREVER_HOME")?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| format!("output clock: {e}"))?.as_nanos();
        let output = new_output(Path::new(&home), stamp, std::process::id()).map_err(|e| format!("fresh run output: {e}"))?;
        write_fixture(&output).map_err(|e| format!("generated fixture: {e}"))?;
        let local = owned_data(&output, data.schema.clone())?;
        field.on = crate::authored::enabled();
        info!("pbr field: evidence directory {}", output.display());
        field.scene = Some(make_scene(world, &local, field.on)?);
        field.output = Some(output);
    }
    let frame = world.resource::<RenderFrames>().0.load(Ordering::Acquire);
    if field.pending.is_none() && elapsed >= Duration::from_secs(5) && frame >= field.after_frame + 10 {
        let target = field.scene.as_ref().ok_or("missing owned scene")?.target.clone();
        let entity = world.spawn((FieldOwned, ReadbackOnce::texture(target))).observe(received).id();
        field.pending = Some(entity);
    }
    Ok(())
}

fn check_layer(world: &mut World) -> Result<(), String> {
    let mut q = world.query_filtered::<(&RenderLayers, Option<&TransmissionAnchor>), Without<FieldOwned>>();
    if q.iter(world).any(|(layers, anchor)| foreign_layer(layers, anchor)) {
        Err("foreign recipient in isolated render layer".into())
    } else {
        Ok(())
    }
}

fn foreign_layer(layers: &RenderLayers, anchor: Option<&TransmissionAnchor>) -> bool {
    anchor.is_none() && layers.intersects(&RenderLayers::layer(LAYER))
}

fn owned_data(out: &Path, schema: Arc<fnv_nif::Schema>) -> Result<GameData, String> {
    let vfs = fnv_vfs::Vfs::open(out, &[]).map_err(|e| format!("owned VFS: {e}"))?;
    for (rel, expected) in fixture() {
        let path = rel.replace('/', "\\");
        let actual = vfs.read(&path).ok_or_else(|| format!("missing owned VFS {path}"))?.map_err(|e| format!("owned VFS {path}: {e}"))?;
        if actual != expected || vfs.source_of(&path) != Some("loose") {
            return Err(format!("different owned VFS payload {path}"));
        }
    }
    Ok(GameData::new(Arc::new(vfs), schema))
}

fn mesh() -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[-1.0f32, -1.0, 0.0], [1.0, -1.0, 0.0], [1.0, 1.0, 0.0], [-1.0, 1.0, 0.0]])
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 0.0, 1.0]; 4])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]])
        .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]))
}

/// The NIF texture paths of each panel's shape (diffuse, then the game's normal map).
fn panel_textures(i: usize) -> Vec<String> {
    let mut t = vec![format!("textures\\field\\{}.dds", NAMES[i])];
    if NAMES[i] == "reference" {
        t.push("textures\\field\\reference_n.dds".into());
    }
    t
}

fn make_scene(world: &mut World, data: &GameData, on: bool) -> Result<Scene, String> {
    let formats = world.get_resource::<CompressedImageFormatSupport>().map(|s| s.0).ok_or("missing the device's compressed format support")?;
    let support = CompressedImageFormatSupport(formats);
    let bc = formats.contains(CompressedImageFormats::BC);
    let mut cache = TextureCache::default();
    let prepared = world.resource_scope(|_, mut images: Mut<Assets<Image>>| -> Result<Vec<(Mesh, NifMaterial)>, String> {
        // The authored `_n` decoded on a worker thread under a reserved handle, as a model
        // prepared there would have it, then adopted by the main thread's build (off: no
        // file under textures\pbr\ is read).
        let worker_n = if on {
            let (provider, workers, snapshot) = (images.get_handle_provider(), cache.workers.clone(), TextureSnapshot::new());
            let decoded = std::thread::scope(|s| {
                let (snapshot, workers, provider) = (&snapshot, workers.clone(), provider.clone());
                s.spawn(move || worker_texture_exact(N_PATH, TextureUsage::NormalTwoChannel, data, snapshot, &workers, &provider, bc)).join()
            })
            .map_err(|_| "worker decode panicked".to_string())?;
            let (handle, format, _) = decoded.ok_or("authored _n not decoded on the worker")?;
            cache.take_workers(&mut images, 0.0);
            Some((handle, format))
        } else {
            None
        };
        let mut prepared = Vec::with_capacity(PANELS);
        for i in 0..PANELS {
            let mut mesh = mesh();
            let mut props = Props::default();
            props.textures = panel_textures(i);
            let (material, missing) = shape_material(&props, &mut mesh, &mut cache, data, &mut images, Some(&support));
            if missing.is_some() || material.base.base_color_texture.is_none() || material.base.unlit {
                return Err(format!("actual NIF material refused {}", NAMES[i]));
            }
            prepared.push((mesh, material));
        }
        let size = |h: &Option<Handle<Image>>| h.as_ref().and_then(|h| images.get(h)).map(|i| (i.size(), i.texture_descriptor.format));
        let m: Vec<&NifMaterial> = prepared.iter().map(|(_, m)| m).collect();
        if on {
            let (worker_handle, worker_format) = worker_n.expect("decoded above when on");
            let b = &m[1].base;
            let orm = b.metallic_roughness_texture.clone();
            if orm.is_none() || b.occlusion_texture != orm || b.specular_texture.is_some() || !crate::authored::is_authored(m[1]) {
                return Err("authored panel: the ORM is not in both slots".into());
            }
            if b.normal_map_texture.as_ref() != Some(&worker_handle) {
                return Err("authored panel: the normal map is not the worker's _n".into());
            }
            let want_format = if bc { TextureFormat::Bc5RgUnorm } else { TextureFormat::Rg8Unorm };
            let (diffuse, orm, normal) = (size(&b.base_color_texture), size(&orm), size(&b.normal_map_texture));
            let sides = [diffuse.map(|d| d.0), orm.map(|o| o.0), normal.map(|n| n.0)];
            if sides != [Some(UVec2::splat(DIFFUSE_SIDE)), Some(UVec2::splat(ORM_SIDE)), Some(UVec2::splat(NORMAL_SIDE))] {
                return Err(format!("authored panel: image sizes {sides:?}"));
            }
            if normal.map(|n| n.1) != Some(want_format) || worker_format != want_format {
                return Err(format!("authored panel: normal format {normal:?}, worker {worker_format:?}, expected {want_format:?}"));
            }
            let c = &m[2].base;
            if c.metallic_roughness_texture.is_none() || c.specular_texture.is_some() || size(&c.normal_map_texture).map(|n| n.1) != Some(TextureFormat::Rgba8Unorm) {
                return Err("reference panel: expected the set's ORM and the three-channel game normal".into());
            }
            if m[3].base.metallic_roughness_texture.is_none() || m[3].base.normal_map_texture.is_some() || m[0].base.metallic_roughness_texture.is_some() {
                return Err("flat or plain panel: unexpected maps".into());
            }
            info!("pbr field: authored set found (orm in both slots, normal {want_format:?} decoded on a worker); sizes diffuse {DIFFUSE_SIDE}x{DIFFUSE_SIDE}, orm {ORM_SIDE}x{ORM_SIDE}, normal {NORMAL_SIDE}x{NORMAL_SIDE}");
        } else {
            if m.iter().any(|m| m.base.metallic_roughness_texture.is_some() || m.base.occlusion_texture.is_some() || crate::authored::is_authored(m)) {
                return Err("setting off: a panel carries an authored map".into());
            }
            if data.pbr_index_built() || PBR_FILES.iter().any(|p| cache.has_or_ahead(p)) {
                return Err("setting off: the index was built or a textures\\pbr\\ file was read".into());
            }
            let plain_fields = |m: &NifMaterial| {
                let mut b = m.base.clone();
                b.base_color_texture = None;
                format!("{b:?}")
            };
            if plain_fields(m[0]) != plain_fields(m[1]) || plain_fields(m[0]) != plain_fields(m[3]) {
                return Err("setting off: the plain, authored and flat materials differ".into());
            }
            info!("pbr field: setting off, no textures\\pbr\\ file read");
        }
        Ok(prepared)
    })?;
    info!("pbr field: actual NIF material source, {PANELS} panels ({}), setting {}, BC {}", NAMES.join(", "), if on { "on" } else { "off" }, if bc { "yes" } else { "no" });
    let mut image = Image::new_target_texture(WIDTH as u32, HEIGHT as u32, TextureFormat::Rgba8Unorm, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = world.resource_mut::<Assets<Image>>().add(image);
    let mut entities = Vec::with_capacity(PANELS + 2);
    for (i, (mesh, material)) in prepared.into_iter().enumerate() {
        let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
        let material = world.resource_mut::<Assets<NifMaterial>>().add(material);
        entities.push(world.spawn((FieldOwned, Name::new(format!("PBR field panel {}", NAMES[i])), Mesh3d(mesh), MeshMaterial3d(material),
            Transform::from_xyz((i as f32 - 1.5) * 2.0, 0.0, 0.0), RenderLayers::layer(LAYER))).id());
    }
    // The light lies in the X-Z plane, so the normal's Y tilt changes N·L the same way
    // whichever way the green channel is read; every panel sees the same light.
    entities.push(world.spawn((FieldOwned, Name::new("PBR field light"),
        DirectionalLight { illuminance: 4.0, shadow_maps_enabled: false, contact_shadows_enabled: false, ..default() },
        Transform::from_translation(Vec3::new(0.6, 0.0, 0.8)).looking_at(Vec3::ZERO, Vec3::Y), RenderLayers::layer(LAYER))).id());
    entities.push(world.spawn((FieldOwned, FieldCamera, Name::new("PBR field camera"), Camera3d::default(),
        Camera { order: -101, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
        RenderTarget::Image(target.clone().into()),
        Projection::Orthographic(OrthographicProjection { scaling_mode: ScalingMode::Fixed { width: (2 * PANELS) as f32, height: 2.0 }, near: 0.1, far: 10.0, ..OrthographicProjection::default_3d() }),
        Transform::from_xyz(0.0, 0.0, 2.0).looking_at(Vec3::ZERO, Vec3::Y),
        Exposure { ev100: 0.0 }, AmbientLight { color: Color::WHITE, brightness: 1.0, ..default() },
        Tonemapping::None, DebandDither::Disabled, Msaa::Off, RenderLayers::layer(LAYER))).id());
    Ok(Scene { entities, target })
}

fn received(
    event: On<ReadbackComplete>, mut field: ResMut<Field>, frames: Res<RenderFrames>,
    layers: Query<(&RenderLayers, Option<&TransmissionAnchor>), Without<FieldOwned>>, mut commands: Commands,
) {
    if field.done || field.pending != Some(event.entity) {
        return;
    }
    field.pending = None;
    commands.entity(event.entity).despawn();
    let ordinal = field.completed + 1;
    let frame = frames.0.load(Ordering::Acquire);
    let result = if layers.iter().any(|(layers, anchor)| foreign_layer(layers, anchor)) {
        Err("foreign recipient in isolated render layer at completion".into())
    } else {
        compare(&event.data, field.on)
    };
    let saved = field.output.as_ref().ok_or_else(|| "missing output directory".to_string())
        .and_then(|out| save_readback(out, ordinal, frame, &event.data, &result).map_err(|e| format!("save readback: {e}")));
    if let Err(reason) = saved.and(result.map(|_| ())) {
        error!("pbr field: FAIL readback {ordinal}: {reason}");
        field.done = true;
        return;
    }
    field.completed = ordinal;
    field.after_frame = frame;
    if ordinal == 1 {
        info!("pbr field: first rendered readback PASS");
    } else {
        info!("pbr field: second rendered readback PASS");
        if field.on {
            info!("pbr field: authored panel differs from the plain one and matches the three-channel reference PASS");
        } else {
            info!("pbr field: setting off, panels identical PASS");
        }
        field.done = true;
    }
}

/// What a readback measured: per panel the mean luminance of its judged region and its
/// brightest channel value there.
#[derive(Debug, PartialEq)]
struct Report {
    mean: [f64; PANELS],
    brightest: [u8; PANELS],
}

fn compare(bytes: &[u8], on: bool) -> Result<Report, String> {
    if bytes.len() != WIDTH * HEIGHT * 4 {
        return Err(format!("wrong byte count {}", bytes.len()));
    }
    if bytes.as_chunks::<4>().0.iter().any(|p| p[3] != 255) {
        return Err("nonopaque target alpha".into());
    }
    let mut report = Report { mean: [0.0; PANELS], brightest: [0; PANELS] };
    for panel in 0..PANELS {
        let mut sum = 0.0;
        for y in ROI {
            for x in ROI {
                let i = (y * WIDTH + panel * PANEL + x) * 4;
                let p = &bytes[i..i + 3];
                sum += 0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2]);
                report.brightest[panel] = report.brightest[panel].max(p[0]).max(p[1]).max(p[2]);
            }
        }
        report.mean[panel] = sum / (ROI.len() * ROI.len()) as f64;
    }
    // Blank or clipped: nothing could be told apart.
    if report.mean.iter().any(|m| !LIT.contains(m)) || report.brightest.iter().any(|&b| b >= CLIP) {
        return Err(format!("blank or clipped panel: {report:?}"));
    }
    let [plain, authored, reference, flat] = report.mean;
    let pass = if on {
        plain - authored >= MIN_OCCLUSION && (authored - reference).abs() <= SAME && (reference - flat).abs() >= MIN_NORMAL
    } else {
        (plain - authored).abs() <= SAME_OFF && (plain - flat).abs() <= SAME_OFF
    };
    if pass { Ok(report) } else { Err(format!("comparator ({}): {report:?}", if on { "on" } else { "off" })) }
}

fn create_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    OpenOptions::new().write(true).create_new(true).open(path)?.write_all(bytes)
}

// A new child per run; earlier evidence is kept. create_dir is the reservation.
fn new_output(home: &Path, stamp: u128, process: u32) -> std::io::Result<PathBuf> {
    let root = home.join("pbr-field-output");
    match fs::create_dir(&root) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let meta = fs::symlink_metadata(&root)?;
            if !meta.is_dir() || meta.file_type().is_symlink() {
                return Err(std::io::Error::other("output parent is not an ordinary directory"));
            }
        }
        Err(e) => return Err(e),
    }
    for attempt in 0..64 {
        let path = root.join(format!("run-{stamp}-{process}-{attempt}"));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("output reservation collision limit"))
}

fn save_readback(out: &Path, ordinal: u8, frame: u64, bytes: &[u8], report: &Result<Report, String>) -> std::io::Result<()> {
    create_file(&out.join(format!("readback-{ordinal}.rgba")), bytes)?;
    create_file(&out.join(format!("readback-{ordinal}.txt")), format!("submitted render frames at receipt: {frame}\npanels: {NAMES:?}\n{report:#?}\n").as_bytes())?;
    if bytes.len() == WIDTH * HEIGHT * 4 {
        let mut ppm = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
        for pixel in bytes.as_chunks::<4>().0 {
            ppm.extend_from_slice(&pixel[..3]);
        }
        create_file(&out.join(format!("readback-{ordinal}.ppm")), &ppm)?;
    }
    Ok(())
}

/// A legacy DDS header (docs/spec-notes/dds.md) for a `side` x `side` texture of one level:
/// 32-bit RGBA in byte order R, G, B, A (masks), or a FourCC block format.
fn header(side: u32, fourcc: Option<&[u8; 4]>) -> Vec<u8> {
    let mut h = vec![0; 128];
    h[..4].copy_from_slice(b"DDS ");
    let pitch = if fourcc.is_some() { 0 } else { side * 4 };
    for (at, n) in [(4, 124u32), (8, 0x1007 | if fourcc.is_some() { 0 } else { 8 }), (12, side), (16, side), (20, pitch), (28, 1), (76, 32), (80, if fourcc.is_some() { 4 } else { 0x41 }), (108, 0x1000)] {
        h[at..at + 4].copy_from_slice(&n.to_le_bytes());
    }
    match fourcc {
        Some(tag) => h[84..88].copy_from_slice(tag),
        None => {
            for (at, n) in [(88, 32u32), (92, 0xff), (96, 0xff00), (100, 0xff0000), (104, 0xff000000)] {
                h[at..at + 4].copy_from_slice(&n.to_le_bytes());
            }
        }
    }
    h
}

/// A uniform `side` x `side` RGBA texture.
fn rgba(side: u32, texel: [u8; 4]) -> Vec<u8> {
    let mut b = header(side, None);
    for _ in 0..side * side {
        b.extend_from_slice(&texel);
    }
    b
}

/// A uniform unsigned BC5 (`ATI2`) texture: each block's red and green endpoints are the
/// texel's, every index 0 [documented: BC5 is two BC4 blocks of 8 bytes, endpoint 0 first].
fn bc5(side: u32, xy: [u8; 2]) -> Vec<u8> {
    let mut b = header(side, Some(b"ATI2"));
    for _ in 0..(side / 4) * (side / 4) {
        b.extend_from_slice(&[xy[0], 0, 0, 0, 0, 0, 0, 0, xy[1], 0, 0, 0, 0, 0, 0, 0]);
    }
    b
}

/// Every generated file, by its path under the output folder.
fn fixture() -> Vec<(&'static str, Vec<u8>)> {
    let diffuse = rgba(DIFFUSE_SIDE, DIFFUSE);
    let orm = rgba(ORM_SIDE, ORM);
    vec![
        ("textures/field/plain.dds", diffuse.clone()),
        ("textures/field/authored.dds", diffuse.clone()),
        ("textures/field/reference.dds", diffuse.clone()),
        ("textures/field/flat.dds", diffuse),
        ("textures/field/reference_n.dds", rgba(4, NORMAL_RGB)),
        ("textures/pbr/field/authored_orm.dds", orm.clone()),
        ("textures/pbr/field/authored_n.dds", bc5(NORMAL_SIDE, NORMAL_XY)),
        ("textures/pbr/field/reference_orm.dds", orm.clone()),
        ("textures/pbr/field/flat_orm.dds", orm),
    ]
}

fn write_fixture(out: &Path) -> std::io::Result<()> {
    fs::create_dir_all(out.join("textures").join("field"))?;
    fs::create_dir_all(out.join("textures").join("pbr").join("field"))?;
    for (rel, bytes) in fixture() {
        create_file(&out.join(rel), &bytes)?;
    }
    create_file(&out.join("scene.txt"), b"4 panels (plain, authored, reference, flat), each a 2x2 quad with normal (0,0,1) and UVs; one directional light from (0.6,0,0.8), illuminance 4; ambient white, brightness 1; exposure ev100 0; no tonemapping\n")
}

#[cfg(test)]
mod tests;
