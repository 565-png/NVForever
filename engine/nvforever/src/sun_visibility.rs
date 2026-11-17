//! Sun visibility (#2670 cloud shadows, #2671 terrain shadows; docs/design/sun-visibility.md):
//! one factor V in [0, 1] on the outdoor sun's direct light, through which other modules
//! darken it. V = K * Vsolid: K the clouds' transmittance, Vsolid the solid occluders'
//! visibility, min(C, T) of the cascade shadow C and the terrain's T, fading to T over the
//! last cascade's end band (sun_visibility.wesl).
//!
//! Producers write [`SunVisibility`] (cloud shadows: `cloud`; terrain shadows: `terrain`).
//! While nothing is set (and no debug view asked for) the plumbing is off: no view has
//! [`NvfSunVisibilityBindings`], so the mesh view layout, the pipelines' shader text and
//! the GPU work are upstream's, nothing is allocated, and the systems here stop at their
//! run conditions.
//!
//! When on:
//! - the sun light ([`crate::sky::Sun`]) outdoors gets [`OutdoorSunLight`], extracted as
//!   bevy_pbr's `NvfOutdoorSun`, which sets `DIRECTIONAL_LIGHT_FLAGS_NVF_OUTDOOR_SUN_BIT`
//!   in its GPU flags: V touches that light only, never the first-person arms' shade light
//!   (player_body.rs), a menu's lights or an interior's directional light (the same `Sun`
//!   entity indoors, unmarked there);
//! - the world camera, the water reflection camera and a scope's lens camera get
//!   [`SunVisibilityView`], extracted as bevy_pbr's `NvfSunVisibilityBindings` (a uniform,
//!   the cloud and terrain textures, a sampler at view bindings 39-42), which gives their
//!   pipelines the shader defs `NVF_SUN_VISIBILITY` (+ `NVF_CLOUD_VISIBILITY`,
//!   `NVF_TERRAIN_VISIBILITY`) and their bind group the bindings (engine/vendor/README.md);
//! - [`HOOK_EDITS`], patched into `apply_pbr_lighting` after lights.rs's display-value
//!   edits, put V in the directional light loop in the place of the sun's shadow, outside
//!   the shadow-receiver condition, so `direct_light`, the display-value `nvf_direct` and
//!   the transmitted lobe all take it, receivers or not (leaves, tree cards, water).
//!
//! Test aids (no settings row yet): `NVF_PLAY_DEBUG_SUN_VISIBILITY=cloud|terrain|combined`
//! draws K, T or V alone in place of the lit colour (before ambient and fog; the image
//! space grade still follows, `--no-grade` leaves it out), and
//! `NVF_PLAY_DEBUG_SUN_VISIBILITY_TEST=checker` fills `cloud` with a generated
//! checkerboard ([`checker_image`]).

use bevy::asset::{load_internal_asset, uuid_handle, RenderAssetUsages};
use bevy::pbr::{NvfOutdoorSun, NvfSunVisibilityBindings};
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{AddressMode, Extent3d, FilterMode, Sampler, SamplerDescriptor, ShaderType, TextureDimension, TextureFormat, UniformBuffer};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::sync_world::RenderEntity;
use bevy::render::texture::{FallbackImage, GpuImage};
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderSystems};
use bevy::shader::Shader;

use crate::fly::FlyCam;
use crate::reflection::ReflectionCamera;
use crate::scope_view::LensCamera;
use crate::sky::{SkyState, Sun};

/// sun_visibility.wesl, imported by the patched `apply_pbr_lighting` as
/// `nvforever::sun_visibility`.
const SHADER: Handle<Shader> = uuid_handle!("5b0f3c52-8f0e-4d55-9d77-2a61c4b7e3a9");

/// What darkens the sun's direct light; producers write it, everything else reads it.
#[derive(Resource, Clone, Default, Debug, PartialEq)]
pub struct SunVisibility {
    /// Cloud shadows (#2670): K.
    pub cloud: Option<CloudVisibility>,
    /// Terrain shadows (#2671): T.
    pub terrain: Option<TerrainVisibility>,
    /// The debug view (`NVF_PLAY_DEBUG_SUN_VISIBILITY`).
    pub debug: DebugMode,
}

impl SunVisibility {
    /// Whether the plumbing is on: a producer is set or a debug view is asked for (a
    /// debug view alone shows V = C, the cascade shadow).
    pub fn active(&self) -> bool {
        self.cloud.is_some() || self.terrain.is_some() || self.debug != DebugMode::Off
    }

    /// The switches the log line names.
    fn mask(&self) -> (bool, bool, DebugMode) {
        (self.cloud.is_some(), self.terrain.is_some(), self.debug)
    }
}

/// The cloud transmittance projected onto the ground along the light (Bevy world axes,
/// game units). A receiver at P lit from L (unit, towards the light) looks up the texture
/// where P + t L meets the plane y = `plane_height`, at uv = hit.xz * `uv_scale` +
/// `uv_offset` (the sampler repeats), and K = 1 - `strength` * (1 - texel). A receiver at
/// or above the plane has K = 1; near the horizon the shadow fades out
/// ([`CLOUD_FADE_FROM`], [`CLOUD_FADE_TO`]).
#[derive(Clone, Debug, PartialEq)]
pub struct CloudVisibility {
    /// Transmittance in the red channel (1 clear, 0 opaque): a filterable float format,
    /// R8Unorm meant; the image's own sampler is not used.
    pub image: Handle<Image>,
    pub uv_scale: Vec2,
    pub uv_offset: Vec2,
    pub plane_height: f32,
    /// 0..1.
    pub strength: f32,
}

/// The terrain's shadow-height map (#2671; its producer and its sampling come with stage
/// T1/T2, this is the binding's contract): a receiver below the map's height at its xz is
/// in the terrain's shadow.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainVisibility {
    pub image: Handle<Image>,
    /// World xz to map uv (uv = xz * scale + offset); outside 0..1 the map says nothing.
    pub uv_scale: Vec2,
    pub uv_offset: Vec2,
    /// View distances over which Vsolid goes from min(C, T) to T: the last cascade's end.
    pub fade_band: Vec2,
    /// The width (uv) over which T fades to 1 at the map's edge.
    pub edge_fade: f32,
    /// The height (game units) over which a receiver goes from shadowed to lit.
    pub height_softness: f32,
}

/// What the debug view draws in place of the lit colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DebugMode {
    #[default]
    Off = 0,
    /// K.
    Cloud = 1,
    /// T.
    Terrain = 2,
    /// V = K * Vsolid.
    Combined = 3,
}

impl DebugMode {
    /// From `NVF_PLAY_DEBUG_SUN_VISIBILITY`; `None` for a value it does not know.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "0" | "off" | "false" | "no" => Some(Self::Off),
            "cloud" => Some(Self::Cloud),
            "terrain" => Some(Self::Terrain),
            "combined" | "1" | "on" => Some(Self::Combined),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Cloud => "cloud",
            Self::Terrain => "terrain",
            Self::Combined => "combined",
        }
    }
}

/// The sine of the light's elevation below which clouds cast no shadow, and from which
/// their shadow is whole: about 1.1 and 5.7 degrees [engineering choice: the hit point
/// runs off towards the horizon as the light gets low; sky.rs fades the sun light itself
/// out within 0.1 radians of the horizon].
pub const CLOUD_FADE_FROM: f32 = 0.02;
pub const CLOUD_FADE_TO: f32 = 0.1;

/// `NvfSunVisibility` of sun_visibility.wesl (field for field).
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct SunVisibilityUniform {
    pub cloud_uv: Vec4,
    pub cloud: Vec4,
    pub terrain_uv: Vec4,
    pub terrain: Vec4,
    pub mode: UVec4,
}

/// Feature bits of `mode.x`.
pub const MASK_CLOUD: u32 = 1;
pub const MASK_TERRAIN: u32 = 2;

impl SunVisibilityUniform {
    pub fn pack(v: &SunVisibility) -> Self {
        let mut u = Self::default();
        let mut mask = 0;
        if let Some(c) = &v.cloud {
            u.cloud_uv = Vec4::new(c.uv_scale.x, c.uv_scale.y, c.uv_offset.x, c.uv_offset.y);
            u.cloud = Vec4::new(c.plane_height, c.strength.clamp(0.0, 1.0), CLOUD_FADE_FROM, CLOUD_FADE_TO);
            mask |= MASK_CLOUD;
        }
        if let Some(t) = &v.terrain {
            u.terrain_uv = Vec4::new(t.uv_scale.x, t.uv_scale.y, t.uv_offset.x, t.uv_offset.y);
            u.terrain = Vec4::new(t.fade_band.x, t.fade_band.y, t.edge_fade.max(0.0), t.height_softness.max(0.0));
            mask |= MASK_TERRAIN;
        }
        u.mode = UVec4::new(mask, v.debug as u32, 0, 0);
        u
    }
}

/// On the light that is the outdoor sun while the plumbing is on (main world).
#[derive(Component, Clone, Copy, Debug)]
pub struct OutdoorSunLight;

/// On the cameras whose views take the sun visibility while the plumbing is on (main
/// world).
#[derive(Component, Clone, Copy, Debug)]
pub struct SunVisibilityView;

/// Test producer: the checkerboard's size in texels, its cells' size in texels and in game
/// units, and its plane (`NVF_PLAY_DEBUG_SUN_VISIBILITY_TEST=checker`).
const CHECKER_TEXELS: u32 = 64;
const CHECKER_CELL_TEXELS: u32 = 8;
pub const CHECKER_CELL: f32 = 2_000.0;
pub const CHECKER_PLANE: f32 = 20_000.0;

/// A transmittance checkerboard: cells of [`CHECKER_CELL_TEXELS`] texels, clear (255) and
/// opaque (0) in turn.
pub fn checker_image() -> Image {
    let n = CHECKER_TEXELS;
    let data = (0..n * n)
        .map(|i| {
            let (x, y) = (i % n, i / n);
            if (x / CHECKER_CELL_TEXELS + y / CHECKER_CELL_TEXELS) % 2 == 0 { 255 } else { 0 }
        })
        .collect();
    Image::new(Extent3d { width: n, height: n, depth_or_array_layers: 1 }, TextureDimension::D2, data, TextureFormat::R8Unorm, RenderAssetUsages::RENDER_WORLD)
}

/// The test producer's cloud: [`checker_image`] with cells of [`CHECKER_CELL`] units,
/// strength 1, on a plane [`CHECKER_PLANE`] units up.
pub fn checker_cloud(image: Handle<Image>) -> CloudVisibility {
    let repeat = CHECKER_CELL * (CHECKER_TEXELS / CHECKER_CELL_TEXELS) as f32;
    CloudVisibility { image, uv_scale: Vec2::splat(1.0 / repeat), uv_offset: Vec2::ZERO, plane_height: CHECKER_PLANE, strength: 1.0 }
}

pub struct SunVisibilityPlugin;

impl Plugin for SunVisibilityPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, SHADER, "sun_visibility.wesl", |s, _| Shader::from_wesl(s, "embedded://nvforever/sun_visibility.wesl"));
        app.init_resource::<SunVisibility>()
            .add_systems(Startup, debug_aids)
            .add_systems(PostUpdate, (log_mask.run_if(resource_changed::<SunVisibility>), mark_targets.run_if(marking_needed)).chain());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_resource::<SunVisibilityGpu>()
            .add_systems(ExtractSchedule, extract_sun_visibility)
            .add_systems(Render, prepare_sun_visibility.in_set(RenderSystems::PrepareResources));
    }
}

/// `NVF_PLAY_DEBUG_SUN_VISIBILITY` and `NVF_PLAY_DEBUG_SUN_VISIBILITY_TEST`.
fn debug_aids(mut vis: ResMut<SunVisibility>, mut images: ResMut<Assets<Image>>) {
    if let Ok(value) = std::env::var("NVF_PLAY_DEBUG_SUN_VISIBILITY") {
        match DebugMode::parse(&value) {
            Some(mode) => vis.debug = mode,
            None => warn!("sun visibility: NVF_PLAY_DEBUG_SUN_VISIBILITY={value} unknown (cloud, terrain or combined)"),
        }
    }
    if let Ok(value) = std::env::var("NVF_PLAY_DEBUG_SUN_VISIBILITY_TEST") {
        if value.trim().eq_ignore_ascii_case("checker") {
            vis.cloud = Some(checker_cloud(images.add(checker_image())));
            info!("sun visibility: test producer: checkerboard clouds, cells {CHECKER_CELL:.0} units, plane {CHECKER_PLANE:.0} units up, strength 1");
        } else if !value.trim().is_empty() {
            warn!("sun visibility: NVF_PLAY_DEBUG_SUN_VISIBILITY_TEST={value} unknown (checker)");
        }
    }
}

/// One line whenever the switches change.
fn log_mask(vis: Res<SunVisibility>, mut last: Local<Option<(bool, bool, DebugMode)>>) {
    let mask = vis.mask();
    if last.unwrap_or((false, false, DebugMode::Off)) != mask {
        let on = |b: bool| if b { "on" } else { "off" };
        info!("sun visibility: cloud {} terrain {} debug {}", on(mask.0), on(mask.1), mask.2.name());
    }
    *last = Some(mask);
}

/// Whether [`mark_targets`] has anything to do: the plumbing is on, or markers are left
/// to take off.
fn marking_needed(vis: Res<SunVisibility>, marked: Query<(), Or<(With<OutdoorSunLight>, With<SunVisibilityView>)>>) -> bool {
    vis.active() || !marked.is_empty()
}

type ViewCandidate = (Entity, Has<SunVisibilityView>, Has<FlyCam>, Has<ReflectionCamera>, Has<LensCamera>);

/// Put [`OutdoorSunLight`] on the sun light while outdoors and [`SunVisibilityView`] on the
/// world cameras while the plumbing is on; take them off otherwise.
fn mark_targets(
    mut commands: Commands,
    vis: Res<SunVisibility>,
    sky: Option<Res<SkyState>>,
    lights: Query<(Entity, Has<Sun>, Has<OutdoorSunLight>), Or<(With<Sun>, With<OutdoorSunLight>)>>,
    views: Query<ViewCandidate, Or<(With<FlyCam>, With<ReflectionCamera>, With<LensCamera>, With<SunVisibilityView>)>>,
) {
    let active = vis.active();
    let outdoors = sky.is_some_and(|s| s.interior.is_none());
    for (e, sun, marked) in &lights {
        let want = active && outdoors && sun;
        if want && !marked {
            commands.entity(e).try_insert(OutdoorSunLight);
        } else if !want && marked {
            commands.entity(e).try_remove::<OutdoorSunLight>();
        }
    }
    for (e, marked, fly, reflection, lens) in &views {
        let want = active && (fly || reflection || lens);
        if want && !marked {
            commands.entity(e).try_insert(SunVisibilityView);
        } else if !want && marked {
            commands.entity(e).try_remove::<SunVisibilityView>();
        }
    }
}

/// Render world: the shared uniform buffer and sampler (made the first time the plumbing
/// is on, then kept), and what this frame's extraction asked for.
#[derive(Resource, Default)]
struct SunVisibilityGpu {
    parts: Option<GpuParts>,
    uniform: SunVisibilityUniform,
    cloud: Option<AssetId<Image>>,
    terrain: Option<AssetId<Image>>,
}

struct GpuParts {
    buffer: UniformBuffer<SunVisibilityUniform>,
    sampler: Sampler,
    written: Option<SunVisibilityUniform>,
}

/// Mirror the markers into the render world: `NvfOutdoorSun` on the sun light's render
/// entity, `NvfSunVisibilityBindings` on the marked views'. The bindings are inserted,
/// changed and removed only here, as bevy_pbr requires (mesh_view_bindings.rs).
#[allow(clippy::too_many_arguments)]
fn extract_sun_visibility(
    mut commands: Commands,
    vis: Extract<Res<SunVisibility>>,
    suns: Extract<Query<RenderEntity, With<OutdoorSunLight>>>,
    views: Extract<Query<RenderEntity, With<SunVisibilityView>>>,
    marked_suns: Query<Entity, With<NvfOutdoorSun>>,
    mut bound: Query<(Entity, &mut NvfSunVisibilityBindings)>,
    mut gpu: ResMut<SunVisibilityGpu>,
    (device, queue, fallback): (Res<RenderDevice>, Res<RenderQueue>, Res<FallbackImage>),
) {
    let active = vis.active();
    if !active && suns.is_empty() && views.is_empty() && marked_suns.is_empty() && bound.is_empty() {
        return;
    }
    for e in &marked_suns {
        if !suns.iter().any(|s| s == e) {
            commands.entity(e).try_remove::<NvfOutdoorSun>();
        }
    }
    for e in &suns {
        if !marked_suns.contains(e) {
            commands.entity(e).try_insert(NvfOutdoorSun);
        }
    }
    for (e, _) in &bound {
        if !views.iter().any(|v| v == e) {
            commands.entity(e).try_remove::<NvfSunVisibilityBindings>();
        }
    }
    if views.is_empty() {
        return;
    }
    gpu.uniform = SunVisibilityUniform::pack(&vis);
    gpu.cloud = vis.cloud.as_ref().map(|c| c.image.id());
    gpu.terrain = vis.terrain.as_ref().map(|t| t.image.id());
    let (cloud, terrain) = (vis.cloud.is_some(), vis.terrain.is_some());
    let uniform = gpu.uniform;
    let parts = gpu.parts.get_or_insert_with(|| {
        let mut buffer = UniformBuffer::from(uniform);
        buffer.set_label(Some("nvf_sun_visibility"));
        buffer.write_buffer(&device, &queue);
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("nvf_sun_visibility"),
            address_mode_u: AddressMode::Repeat,
            address_mode_v: AddressMode::Repeat,
            address_mode_w: AddressMode::Repeat,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..default()
        });
        GpuParts { buffer, sampler, written: Some(uniform) }
    });
    let Some(buffer) = parts.buffer.buffer() else { return };
    for e in &views {
        if let Ok((_, mut b)) = bound.get_mut(e) {
            if b.cloud != cloud || b.terrain != terrain {
                b.cloud = cloud;
                b.terrain = terrain;
            }
        } else {
            commands.entity(e).try_insert(NvfSunVisibilityBindings {
                uniform: buffer.clone(),
                cloud_texture: fallback.d2.texture_view.clone(),
                terrain_texture: fallback.d2.texture_view.clone(),
                sampler: parts.sampler.clone(),
                cloud,
                terrain,
            });
        }
    }
}

/// Write the uniform when it changed and point the views at this frame's textures (a
/// texture not uploaded yet: the white fallback, which lets the light through).
fn prepare_sun_visibility(
    mut gpu: ResMut<SunVisibilityGpu>,
    mut bound: Query<&mut NvfSunVisibilityBindings>,
    images: Res<RenderAssets<GpuImage>>,
    fallback: Res<FallbackImage>,
    (device, queue): (Res<RenderDevice>, Res<RenderQueue>),
) {
    if bound.is_empty() {
        return;
    }
    let gpu = &mut *gpu;
    let Some(parts) = gpu.parts.as_mut() else { return };
    if parts.written != Some(gpu.uniform) {
        parts.buffer.set(gpu.uniform);
        parts.buffer.write_buffer(&device, &queue);
        parts.written = Some(gpu.uniform);
    }
    let view = |id: Option<AssetId<Image>>| id.and_then(|id| images.get(id)).map_or(&fallback.d2.texture_view, |g| &g.texture_view);
    let (cloud, terrain) = (view(gpu.cloud), view(gpu.terrain));
    for mut b in &mut bound {
        if b.cloud_texture.id() != cloud.id() {
            b.cloud_texture = cloud.clone();
        }
        if b.terrain_texture.id() != terrain.id() {
            b.terrain_texture = terrain.clone();
        }
    }
}

/// The hook in bevy_pbr's `apply_pbr_lighting` [observed: bevy_pbr 0.20.1
/// `pbr_functions.wesl` with lights.rs `DISPLAY_EDITS` made], in order, each at the first
/// occurrence of its anchor after the last edit: (anchor, text, where the text goes).
/// Every statement it adds is under `@if(NVF_SUN_VISIBILITY)`, and the fog block is kept
/// word for word under `@if(DISTANCE_FOG && !NVF_SUN_VISIBILITY)`, so without the shader
/// def the text a pipeline compiles is the display-value patch's.
pub const HOOK_EDITS: [(&str, &str, Place); 6] = [
    (
        "import bevy_render::maths::{E, powsafe};\n",
        "// NVForever sun visibility (nvforever sun_visibility.rs).
@if(NVF_SUN_VISIBILITY) import nvforever::sun_visibility::{nvf_sun_visibility, nvf_sun_visibility_parts, nvf_sun_debug_value, nvf_sun_debug_mode};
",
        Place::After,
    ),
    (
        "    // directional lights (direct)\n",
        "    // NVForever sun visibility: what the debug view shows (nvforever sun_visibility.rs).
    @if(NVF_SUN_VISIBILITY)
    var nvf_sun_debug = 1.0;

",
        Place::Before,
    ),
    (
        "        var light_contrib = lighting::directional_light(i, &lighting_input, enable_diffuse);\n",
        "        // NVForever sun visibility (nvforever sun_visibility.rs): the outdoor sun's shadow
        // becomes V = K * Vsolid, outside the shadow-receiver condition, so that receivers
        // and non-receivers (leaves, tree cards, water) alike take the clouds and terrain.
        @if(NVF_SUN_VISIBILITY)
        if ((*light).flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_NVF_OUTDOOR_SUN_BIT) != 0u {
            let nvf_parts = nvf_sun_visibility_parts(in.world_position.xyz, (*light).direction_to_light, shadow, view_z);
            nvf_sun_debug = nvf_sun_debug_value(nvf_parts);
            shadow = nvf_parts.w;
        }
",
        Place::Before,
    ),
    (
        "        let transmitted_light_contrib =\n            lighting::directional_light(i, &transmissive_lighting_input, enable_diffuse);\n",
        "        // NVForever sun visibility: the transmitted lobe takes it too.
        @if(NVF_SUN_VISIBILITY)
        if ((*light).flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_NVF_OUTDOOR_SUN_BIT) != 0u {
            transmitted_shadow = nvf_sun_visibility(diffuse_transmissive_lobe_world_position.xyz, (*light).direction_to_light, transmitted_shadow, view_z);
        }
",
        Place::Before,
    ),
    (
        "    return output_color;\n}\n",
        "    // NVForever sun visibility: the debug view draws the factor alone.
    @if(NVF_SUN_VISIBILITY)
    if nvf_sun_debug_mode() != 0u {
        output_color = vec4<f32>(vec3<f32>(nvf_sun_debug), output_color.a);
    }

",
        Place::Before,
    ),
    (FOG_BLOCK, FOG_BLOCK_HOOKED, Place::Replace),
];

/// Where an edit's text goes against its anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Before,
    After,
    Replace,
}

/// `main_pass_post_lighting_processing`'s fog [observed: bevy_pbr 0.20.1].
const FOG_BLOCK: &str = "    @if(DISTANCE_FOG)
    // fog
    if ((pbr_input.material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT) != 0u) {
        output_color = apply_fog(
            view_bindings::fog,
            output_color,
            pbr_input.world_position.xyz,
            view_bindings::view.world_position.xyz,
            pbr_input.frag_coord.xy,
        );
    }
";

/// The same, left out while the sun visibility's debug view is drawn.
const FOG_BLOCK_HOOKED: &str = "    @if(DISTANCE_FOG && !NVF_SUN_VISIBILITY)
    // fog
    if ((pbr_input.material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT) != 0u) {
        output_color = apply_fog(
            view_bindings::fog,
            output_color,
            pbr_input.world_position.xyz,
            view_bindings::view.world_position.xyz,
            pbr_input.frag_coord.xy,
        );
    }
    // NVForever sun visibility: no fog over the debug view (nvforever sun_visibility.rs).
    @if(DISTANCE_FOG && NVF_SUN_VISIBILITY)
    if ((pbr_input.material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT) != 0u && nvf_sun_debug_mode() == 0u) {
        output_color = apply_fog(
            view_bindings::fog,
            output_color,
            pbr_input.world_position.xyz,
            view_bindings::view.world_position.xyz,
            pbr_input.frag_coord.xy,
        );
    }
";

/// `source` (lights.rs's display-value patch made) with [`HOOK_EDITS`]; `None` when an
/// anchor is missing (Bevy changed), and the caller keeps `source`.
pub fn hook_patched(source: &str) -> Option<String> {
    let mut out = String::with_capacity(source.len() + 4096);
    let mut rest = source;
    for (anchor, text, place) in HOOK_EDITS {
        let at = rest.find(anchor)?;
        out.push_str(&rest[..at]);
        match place {
            Place::Before => {
                out.push_str(text);
                out.push_str(anchor);
            }
            Place::After => {
                out.push_str(anchor);
                out.push_str(text);
            }
            Place::Replace => out.push_str(text),
        }
        rest = &rest[at + anchor.len()..];
    }
    out.push_str(rest);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lights::unix_lines;

    fn vendored_functions() -> String {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/bevy_pbr/src/render/pbr_functions.wesl");
        unix_lines(&std::fs::read_to_string(path).expect("vendored pbr_functions.wesl"))
    }

    /// The source with the display-value patch, then the hook.
    fn hooked() -> (String, String) {
        let display = crate::lights::display_patched(&vendored_functions()).expect("display patch applies");
        let hooked = hook_patched(&display).expect("hook applies");
        (display, hooked)
    }

    /// With nothing enabled the lighting is untouched: no marker goes anywhere, and the
    /// hook adds only statements under `@if(NVF_SUN_VISIBILITY)` (taking them out gives the
    /// display-value patch back word for word), so the shader text without the def is the
    /// old one.
    #[test]
    fn visibility_identity_preserves_lighting() {
        let (display, hooked) = hooked();
        // Each added text is comments and statements under the def.
        for (_, text, place) in HOOK_EDITS {
            if place == Place::Replace {
                continue;
            }
            let code: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("//")).collect();
            assert!(code[0].starts_with("@if(NVF_SUN_VISIBILITY)"), "{text}");
        }
        // The fog block is kept under `!NVF_SUN_VISIBILITY` and its twin is under the def.
        assert!(FOG_BLOCK_HOOKED.contains(&FOG_BLOCK.replace("@if(DISTANCE_FOG)", "@if(DISTANCE_FOG && !NVF_SUN_VISIBILITY)")));
        assert!(FOG_BLOCK_HOOKED.contains("@if(DISTANCE_FOG && NVF_SUN_VISIBILITY)"));
        // Taking the additions out again gives the display-value patch.
        let mut back = hooked.clone();
        for (anchor, text, place) in HOOK_EDITS {
            match place {
                Place::Before => back = back.replacen(&format!("{text}{anchor}"), anchor, 1),
                Place::After => back = back.replacen(&format!("{anchor}{text}"), anchor, 1),
                Place::Replace => back = back.replacen(text, anchor, 1),
            }
        }
        assert_eq!(back, display);

        // No producer, no debug view: the plumbing is off and nothing is marked.
        let mut app = App::new();
        app.insert_resource(SunVisibility::default()).insert_resource(SkyState::default()).add_systems(Update, mark_targets.run_if(marking_needed));
        let sun = app.world_mut().spawn((Sun, DirectionalLight::default())).id();
        let cam = app.world_mut().spawn(FlyCam { yaw: 0.0, pitch: 0.0, speed: 0.0 }).id();
        app.update();
        assert!(!SunVisibility::default().active());
        assert!(app.world().get::<OutdoorSunLight>(sun).is_none());
        assert!(app.world().get::<SunVisibilityView>(cam).is_none());
        assert_eq!(SunVisibilityUniform::pack(&SunVisibility::default()), SunVisibilityUniform::default());
    }

    /// Only the sun light outdoors takes the factor: not the arms' shade light or a menu's
    /// light, not the same light indoors; the world, reflection and scope views take the
    /// bindings, a menu's camera does not; switching off takes every marker off again.
    #[test]
    fn visibility_targets_outdoor_light() {
        let mut app = App::new();
        let images = Handle::<Image>::default();
        app.insert_resource(SunVisibility { cloud: Some(checker_cloud(images)), ..default() })
            .insert_resource(SkyState::default())
            .add_systems(Update, mark_targets.run_if(marking_needed));
        let sun = app.world_mut().spawn((Sun, DirectionalLight::default())).id();
        let shade = app.world_mut().spawn(DirectionalLight::default()).id();
        let world_cam = app.world_mut().spawn(FlyCam { yaw: 0.0, pitch: 0.0, speed: 0.0 }).id();
        let mirror = app.world_mut().spawn(ReflectionCamera).id();
        let lens = app.world_mut().spawn(LensCamera).id();
        let menu = app.world_mut().spawn(Name::new("Lockpicking camera")).id();
        app.update();
        let w = app.world();
        assert!(w.get::<OutdoorSunLight>(sun).is_some());
        assert!(w.get::<OutdoorSunLight>(shade).is_none());
        for cam in [world_cam, mirror, lens] {
            assert!(w.get::<SunVisibilityView>(cam).is_some());
        }
        assert!(w.get::<SunVisibilityView>(menu).is_none());

        // Indoors the same light is the cell's directional light: unmarked; the views keep
        // their bindings (no new pipelines at every door).
        app.world_mut().resource_mut::<SkyState>().interior = Some(Default::default());
        app.update();
        assert!(app.world().get::<OutdoorSunLight>(sun).is_none());
        assert!(app.world().get::<SunVisibilityView>(world_cam).is_some());

        // Outdoors again, then everything off.
        app.world_mut().resource_mut::<SkyState>().interior = None;
        app.update();
        assert!(app.world().get::<OutdoorSunLight>(sun).is_some());
        *app.world_mut().resource_mut::<SunVisibility>() = SunVisibility::default();
        app.update();
        let w = app.world();
        assert!(w.get::<OutdoorSunLight>(sun).is_none());
        for cam in [world_cam, mirror, lens] {
            assert!(w.get::<SunVisibilityView>(cam).is_none());
        }
    }

    #[test]
    fn hook_sits_in_the_sun_loop_outside_the_receiver_condition() {
        let (_, out) = hooked();
        let at = |s: &str| out.find(s).unwrap_or_else(|| panic!("{s} missing"));
        let directional = at("    // directional lights (direct)\n");
        let receiver = directional + out[directional..].find("shadow *= in.directional_shadow_factor;\n        }\n").expect("receiver block");
        let hook = at("shadow = nvf_parts.w;");
        let contrib = at("var light_contrib = lighting::directional_light(i, &lighting_input");
        let display = at("nvf_directional_display(i, in.N) * shadow");
        let transmitted_hook = at("transmitted_shadow = nvf_sun_visibility(");
        let transmitted_sum = transmitted_hook + out[transmitted_hook..].find("transmitted_light += transmitted_light_contrib * transmitted_shadow;").expect("transmitted sum");
        let debug = at("output_color = vec4<f32>(vec3<f32>(nvf_sun_debug)");
        let total = at("    // Total light\n");
        // After the receiver condition has closed, before both sums of the light.
        assert!(directional < receiver && receiver < hook && hook < contrib && contrib < display);
        assert!(display < transmitted_hook && transmitted_hook < transmitted_sum);
        assert!(transmitted_sum < total && total < debug);
        assert!(debug < at("@if(DISTANCE_FOG && NVF_SUN_VISIBILITY)"));
        assert!(at("@if(NVF_SUN_VISIBILITY) import nvforever::sun_visibility::") < at("fn nvf_apply_pbr_lighting("));
        assert_eq!(out.matches("nvf_sun_visibility_parts(").count(), 1, "called once");
        // A missing anchor (Bevy changed) leaves the source alone.
        assert!(hook_patched(&vendored_functions().replace("    // directional lights (direct)\n", "")).is_none());
    }

    #[test]
    fn uniform_packing() {
        assert_eq!(SunVisibilityUniform::min_size().get(), 80, "five vec4s, as sun_visibility.wesl's struct");
        let image = Handle::<Image>::default();
        let v = SunVisibility {
            cloud: Some(CloudVisibility { image: image.clone(), uv_scale: Vec2::new(0.5, 0.25), uv_offset: Vec2::new(3.0, 4.0), plane_height: 9_000.0, strength: 1.5 }),
            terrain: Some(TerrainVisibility { image, uv_scale: Vec2::splat(2.0), uv_offset: Vec2::new(-1.0, 1.0), fade_band: Vec2::new(7_000.0, 8_000.0), edge_fade: 0.05, height_softness: 64.0 }),
            debug: DebugMode::Terrain,
        };
        let u = SunVisibilityUniform::pack(&v);
        assert_eq!(u.cloud_uv, Vec4::new(0.5, 0.25, 3.0, 4.0));
        assert_eq!(u.cloud, Vec4::new(9_000.0, 1.0, CLOUD_FADE_FROM, CLOUD_FADE_TO), "strength held to 1");
        assert_eq!(u.terrain_uv, Vec4::new(2.0, 2.0, -1.0, 1.0));
        assert_eq!(u.terrain, Vec4::new(7_000.0, 8_000.0, 0.05, 64.0));
        assert_eq!(u.mode, UVec4::new(MASK_CLOUD | MASK_TERRAIN, 2, 0, 0));
        // The WGSL struct names the same fields in the same order.
        let wesl = include_str!("sun_visibility.wesl");
        let fields: Vec<usize> = ["cloud_uv: vec4<f32>", "cloud: vec4<f32>", "terrain_uv: vec4<f32>", "terrain: vec4<f32>", "mode: vec4<u32>"].iter().map(|f| wesl.find(f).unwrap_or_else(|| panic!("{f}"))).collect();
        assert!(fields.windows(2).all(|w| w[0] < w[1]));
        // Bindings as bevy_pbr's layout has them (mesh_view_bindings.rs, 39-42).
        for b in ["@group(0) @binding(39) var<uniform> nvf_sun", "@group(0) @binding(40) var nvf_cloud_texture: texture_2d<f32>", "@group(0) @binding(41) var nvf_terrain_texture: texture_2d<f32>", "@group(0) @binding(42) var nvf_sun_visibility_sampler: sampler"] {
            assert!(wesl.contains(b), "{b}");
        }
    }

    #[test]
    fn debug_modes_parse() {
        assert_eq!(DebugMode::parse("Cloud"), Some(DebugMode::Cloud));
        assert_eq!(DebugMode::parse(" terrain "), Some(DebugMode::Terrain));
        assert_eq!(DebugMode::parse("combined"), Some(DebugMode::Combined));
        assert_eq!(DebugMode::parse("off"), Some(DebugMode::Off));
        assert_eq!(DebugMode::parse("cascade"), None);
        assert!(SunVisibility { debug: DebugMode::Combined, ..default() }.active());
    }

    #[test]
    fn checker_cells_are_2000_units() {
        let image = checker_image();
        let cloud = checker_cloud(Handle::default());
        let data = image.data.as_ref().expect("pixels");
        let n = CHECKER_TEXELS as f32;
        // Texel centre of a world point (xz) through the cloud's uv, repeating.
        let at = |x: f32, z: f32| {
            let uv = Vec2::new(x, z) * cloud.uv_scale + cloud.uv_offset;
            let (u, v) = (((uv.x.rem_euclid(1.0)) * n) as usize, ((uv.y.rem_euclid(1.0)) * n) as usize);
            data[v * CHECKER_TEXELS as usize + u]
        };
        let c = CHECKER_CELL;
        assert_ne!(at(0.5 * c, 0.5 * c), at(1.5 * c, 0.5 * c));
        assert_ne!(at(0.5 * c, 0.5 * c), at(0.5 * c, 1.5 * c));
        assert_eq!(at(0.5 * c, 0.5 * c), at(1.5 * c, 1.5 * c));
        assert_eq!(at(0.1 * c, 0.1 * c), at(0.9 * c, 0.9 * c));
        assert_eq!((cloud.plane_height, cloud.strength), (CHECKER_PLANE, 1.0));
    }

    /// sun_visibility.wesl in each of its four variants (cloud, terrain on or off) is valid
    /// WGSL: the conditions in front of its functions are resolved by hand here.
    #[test]
    fn shader_variants_are_valid_wgsl() {
        use wgpu::naga;
        let source = include_str!("sun_visibility.wesl").replace("\r\n", "\n");
        for (cloud, terrain) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut out = String::new();
            let mut skipping = false;
            for line in source.lines() {
                if skipping {
                    skipping = line != "}";
                    continue;
                }
                if let Some(cond) = line.strip_prefix("@if(").and_then(|l| l.strip_suffix(')')) {
                    let (negated, name) = cond.strip_prefix('!').map_or((false, cond), |n| (true, n));
                    let on = match name {
                        "NVF_CLOUD_VISIBILITY" => cloud,
                        "NVF_TERRAIN_VISIBILITY" => terrain,
                        other => panic!("unexpected condition {other}"),
                    };
                    skipping = on == negated;
                    continue;
                }
                out.push_str(line);
                out.push('\n');
            }
            let module = naga::front::wgsl::parse_str(&out).unwrap_or_else(|e| panic!("{cloud} {terrain}: {}", e.emit_to_string(&out)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty()).validate(&module).unwrap_or_else(|e| panic!("{cloud} {terrain}: {e:?}"));
            assert_eq!(out.matches("fn nvf_cloud_transmittance(").count(), 1);
            assert_eq!(out.matches("fn nvf_terrain_visibility(").count(), 1);
            assert_eq!(out.matches("fn nvf_solid_visibility(").count(), 1);
        }
    }
}
