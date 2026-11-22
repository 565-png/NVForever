//! HDR output on an HDR screen [engineering choice]: the Display page's HDR rows (on at the
//! next start; paper white and peak brightness live), or `NVF_HDR=1` for a run.
//!
//! - The window's surface is `Rgba16Float`, which wgpu's `SurfaceColorSpace::Auto` makes
//!   scRGB (vendored bevy_render `view/window/mod.rs`).
//! - The main view stays fp16 (graphics.rs), so nothing is cut to 8 bits on the way.
//! - Before the HUD, [`hdr_expand`] (`hdr_expand.wesl`) keeps the graded, tonemapped
//!   picture as it is up to a knee (default 0.6) and lets what lies above it reach the
//!   screen's peak (`NVF_HDR_PEAK` nits, default 600). The game's look is unchanged below
//!   the knee: sun, fire, neon, lightning and bright sky get the extra range.
//! - The final blit to the window (vendored bevy_core_pipeline `blit`) multiplies by paper
//!   white / 80 (`NVF_HDR_PAPER_WHITE` nits, default 200; scRGB's 1.0 is 80 nits), so the
//!   picture's 1.0 and the HUD show at paper white.

use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::prelude::*;
use bevy::render::render_resource::binding_types::{sampler, texture_2d};
use bevy::render::render_resource::{
    AddressMode, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId, ColorTargetState, ColorWrites, FilterMode,
    FragmentState, Operations, PipelineCache, RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, Sampler, SamplerBindingType,
    SamplerDescriptor, ShaderStages, TextureFormat, TextureSampleType,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::view::ViewTarget;
use bevy::render::{RenderApp, RenderStartup};

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};

/// The row's choice (the INI keeps it; it takes effect at the next start).
static WANTED: AtomicBool = AtomicBool::new(false);
/// The first frame has run: the window's surface is made, later choices wait for a restart.
static STARTED: AtomicBool = AtomicBool::new(false);
/// Peak brightness in nits (the row's slider); 0 takes the screen's own (`AUTO_PEAK`).
static PEAK_NITS: AtomicU32 = AtomicU32::new(0);
/// The HDR screen's peak as Windows reports it, read at start; 0 when none reported.
static AUTO_PEAK: AtomicU32 = AtomicU32::new(0);
/// The peak used when neither the slider nor Windows gives one.
const FALLBACK_PEAK: u32 = 600;
/// How far bright light keeps the SDR picture's hue, x1000 (the row's slider).
static HUE_MILLI: AtomicU32 = AtomicU32::new(1000);
/// How far the top of the SDR range (above the knee) is spread into the HDR range, x1000
/// (the row's slider): at 1000 the SDR picture's white reaches 60 % of the way from paper
/// white to the peak, leaving the rest to light sources.
static EXPAND_MILLI: AtomicU32 = AtomicU32::new(500);
/// The gain on light past the SDR picture's white (sun, fire, neon, lamps), x10.
static SOURCE_GAIN_TENTHS: AtomicU32 = AtomicU32::new(60);
/// Where the spread of the SDR range starts (brightest channel).
const EXPAND_KNEE: f32 = 0.6;
/// The brightness heatmap in place of the picture (the row's switch).
static HEATMAP: AtomicBool = AtomicBool::new(false);

/// Whether this run asked for HDR output (the row at start, or `NVF_HDR=1`).
pub fn on() -> bool {
    bevy::render::view::window::NVF_HDR_SURFACE.load(Relaxed) || std::env::var("NVF_HDR").is_ok_and(|v| v.trim() == "1")
}

fn set_wanted(on: bool) {
    WANTED.store(on, Relaxed);
    if !STARTED.load(Relaxed) {
        bevy::render::view::window::NVF_HDR_SURFACE.store(on, Relaxed);
    }
}

fn paper_nits() -> u32 {
    bevy::core_pipeline::upscaling::NVF_HDR_PAPER_WHITE_NITS.load(Relaxed)
}
fn hud_nits() -> u32 {
    bevy::core_pipeline::upscaling::NVF_HDR_HUD_NITS.load(Relaxed)
}
fn hud_part() -> f32 {
    (hud_nits().clamp(80, 400) - 80) as f32 / 320.0
}
fn set_hud_part(v: f32) {
    bevy::core_pipeline::upscaling::NVF_HDR_HUD_NITS.store(80 + ((v.clamp(0.0, 1.0) * 32.0).round() as u32) * 10, Relaxed);
}

/// Paper white slider: 80 to 400 nits in steps of 10.
fn paper_part() -> f32 {
    (paper_nits().clamp(80, 400) - 80) as f32 / 320.0
}
fn set_paper_part(v: f32) {
    let nits = 80 + ((v.clamp(0.0, 1.0) * 32.0).round() as u32) * 10;
    bevy::core_pipeline::upscaling::NVF_HDR_PAPER_WHITE_NITS.store(nits, Relaxed);
}
/// The peak in nits now: the slider's, else the screen's, else [`FALLBACK_PEAK`].
fn peak_nits() -> u32 {
    match PEAK_NITS.load(Relaxed) {
        0 => match AUTO_PEAK.load(Relaxed) {
            0 => FALLBACK_PEAK,
            n => n,
        },
        n => n,
    }
}
/// Peak slider: Auto at the left end, then 400 to 2000 nits in steps of 50.
fn peak_part() -> f32 {
    match PEAK_NITS.load(Relaxed) {
        0 => 0.0,
        n => ((n.clamp(400, 2000) - 400) / 50 + 1) as f32 / 33.0,
    }
}
fn set_peak_part(v: f32) {
    let k = (v.clamp(0.0, 1.0) * 33.0).round() as u32;
    PEAK_NITS.store(if k == 0 { 0 } else { 400 + (k - 1) * 50 }, Relaxed);
}
fn peak_shown(v: f32) -> String {
    let k = (v * 33.0).round() as u32;
    if k > 0 {
        return nits_shown(400 + (k - 1) * 50);
    }
    match AUTO_PEAK.load(Relaxed) {
        0 => format!("Auto ({FALLBACK_PEAK} nits)"),
        n => format!("Auto ({n} nits)"),
    }
}

/// The brightest HDR screen's peak and black level in nits as Windows reports them (from
/// the screen's EDID, or the Windows HDR Calibration app's profile) [documented:
/// IDXGIOutput6::GetDesc1, DXGI_OUTPUT_DESC1]. Only outputs in HDR mode (PQ, BT.2020)
/// count. `device`: the screen the window is on, by its device name ("\\.\DISPLAY1"); its
/// figures if it is in HDR mode, none if not. Without it, or when no output has that
/// name, the brightest HDR output's.
#[cfg(windows)]
fn display_peak(device: Option<&str>) -> Option<(f32, f32)> {
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::Common::DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput6};
    // SAFETY: creates a COM object that the returned wrapper owns and releases.
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.ok()?;
    let mut best: Option<(f32, f32)> = None;
    let mut a = 0;
    // SAFETY: `factory` is a live DXGI factory; the call only reads the index.
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(a) } {
        a += 1;
        let mut o = 0;
        // SAFETY: `adapter` is a live DXGI adapter; the call only reads the index.
        while let Ok(output) = unsafe { adapter.EnumOutputs(o) } {
            o += 1;
            let Ok(output6) = output.cast::<IDXGIOutput6>() else { continue };
            // SAFETY: `output6` is a live DXGI output; the description is returned by value.
            let Ok(desc) = (unsafe { output6.GetDesc1() }) else { continue };
            let hdr = desc.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;
            let name = String::from_utf16_lossy(&desc.DeviceName);
            if device.is_some_and(|d| d == name.trim_end_matches('\0')) {
                return hdr.then_some((desc.MaxLuminance, desc.MinLuminance));
            }
            if hdr && desc.MaxLuminance > best.map_or(0.0, |b| b.0) {
                best = Some((desc.MaxLuminance, desc.MinLuminance));
            }
        }
    }
    best
}
#[cfg(not(windows))]
fn display_peak(_device: Option<&str>) -> Option<(f32, f32)> {
    None
}

/// Auto peak follows the screen the window is on: read again from Windows whenever the
/// window's monitor changes. winit names a monitor by the device name DXGI reports
/// ("\\.\DISPLAY1") [hypothesis: both come from GetMonitorInfo's szDevice].
fn follow_monitor(windows: Query<&bevy::window::OnMonitor, With<bevy::window::PrimaryWindow>>, monitors: Query<&bevy::window::Monitor>, mut last: Local<Option<String>>) {
    if !on() {
        return;
    }
    let Some(name) = windows.single().ok().and_then(|m| monitors.get(m.0).ok()).and_then(|m| m.name.clone()) else { return };
    if last.as_deref() == Some(name.as_str()) {
        return;
    }
    match display_peak(Some(&name)) {
        Some((peak, black)) if peak >= 100.0 => {
            AUTO_PEAK.store(peak.round() as u32, Relaxed);
            info!("hdr: the window's screen {name} reports a peak of {peak:.0} nits, black {black:.3} nits");
        }
        _ => {
            AUTO_PEAK.store(0, Relaxed);
            info!("hdr: the window's screen {name} reports no HDR peak; Auto uses {FALLBACK_PEAK} nits");
        }
    }
    *last = Some(name);
}
fn nits_shown(nits: u32) -> String {
    format!("{nits} nits")
}

/// Logs the output once the window's surface is made (or 300 frames on, without one).
fn mark_started(mut frames: Local<u32>) {
    if *frames == 0 {
        STARTED.store(true, Relaxed);
    }
    if *frames == u32::MAX {
        return;
    }
    *frames += 1;
    if bevy::render::view::window::NVF_HDR_SURFACE_ACTIVE.load(Relaxed) || !on() || *frames >= 300 {
        *frames = u32::MAX;
        info!("hdr: output {}", if bevy::render::view::window::NVF_HDR_SURFACE_ACTIVE.load(Relaxed) { "on (scRGB)" } else if on() { "asked for, but the screen's surface has no fp16 scRGB" } else { "off" });
    }
}

pub struct HdrPlugin;

impl Plugin for HdrPlugin {
    fn build(&self, app: &mut App) {
        use nvf_play::settings::{add_setting, Setting, Slider};
        if let Ok(v) = std::env::var("NVF_HDR_PAPER_WHITE") {
            if let Ok(n) = v.trim().parse::<u32>() {
                bevy::core_pipeline::upscaling::NVF_HDR_PAPER_WHITE_NITS.store(n.clamp(80, 1000), Relaxed);
            }
        }
        if let Some(n) = std::env::var("NVF_HDR_PEAK").ok().and_then(|v| v.trim().parse::<u32>().ok()) {
            PEAK_NITS.store(n.clamp(80, 4000), Relaxed);
        }
        if std::env::var("NVF_HDR_HEATMAP").is_ok_and(|v| v.trim() == "1") {
            HEATMAP.store(true, Relaxed);
        }
        if on() {
            match display_peak(None) {
                Some((peak, black)) if peak >= 100.0 => {
                    AUTO_PEAK.store(peak.round() as u32, Relaxed);
                    info!("hdr: the screen reports a peak of {peak:.0} nits, black {black:.3} nits");
                }
                _ => info!("hdr: no HDR screen reports its peak; Auto uses {FALLBACK_PEAK} nits"),
            }
        }
        add_setting(app, Setting {
            key: "hdr-output",
            group: "Display",
            label: "HDR output",
            note: "On: the game is drawn for an HDR screen (Windows HDR on; on a laptop, its discrete graphics mode). Takes effect at the next start.",
            switches: "",
            get: |_| WANTED.load(Relaxed),
            set: |_, on| set_wanted(on),
            slider: None,
        });
        add_setting(app, Setting {
            key: "hdr-paper-white",
            group: "Display",
            label: "HDR paper white",
            note: "How bright white paper and the HUD look in HDR. Match it to Windows' SDR content brightness.",
            switches: "",
            get: |_| false,
            set: |_, _| {},
            slider: Some(Slider { get: |_| paper_part(), set: |_, v| set_paper_part(v), min: 0.0, step: 1.0 / 32.0, shown: Some(|v| nits_shown(80 + ((v * 32.0).round() as u32) * 10)) }),
        });
        add_setting(app, Setting {
            key: "hdr-peak",
            group: "Display",
            label: "HDR peak brightness",
            note: "The brightest the screen shows (sun, fire, neon, lightning). Auto takes what Windows reports for the screen (run the Windows HDR Calibration app for an exact figure). Else raise it until the sun's disc stops getting brighter.",
            switches: "",
            get: |_| false,
            set: |_, _| {},
            slider: Some(Slider { get: |_| peak_part(), set: |_, v| set_peak_part(v), min: 0.0, step: 1.0 / 33.0, shown: Some(peak_shown) }),
        });
        add_setting(app, Setting {
            key: "hdr-hud-brightness",
            group: "Display",
            label: "HDR HUD brightness",
            note: "How bright the HUD and menus look in HDR, apart from the game picture.",
            switches: "",
            get: |_| false,
            set: |_, _| {},
            slider: Some(Slider { get: |_| hud_part(), set: |_, v| set_hud_part(v), min: 0.0, step: 1.0 / 32.0, shown: Some(|v| nits_shown(80 + ((v * 32.0).round() as u32) * 10)) }),
        });
        add_setting(app, Setting {
            key: "hdr-gamma-22",
            group: "Display",
            label: "HDR gamma 2.2",
            note: "On: dark scenes look as on an SDR monitor (gamma 2.2), as the game was made for. Off: the sRGB curve, with lighter shadows.",
            switches: "",
            get: |_| bevy::core_pipeline::upscaling::NVF_HDR_GAMMA22.load(Relaxed),
            set: |_, on| bevy::core_pipeline::upscaling::NVF_HDR_GAMMA22.store(on, Relaxed),
            slider: None,
        });
        add_setting(app, Setting {
            key: "hdr-highlight-hue",
            group: "Display",
            label: "HDR highlight colour",
            note: "100 %: light brighter than paper white keeps the colour the SDR picture gives it. 0 %: the light's own colour, which can drift (orange fire towards red).",
            switches: "",
            get: |_| false,
            set: |_, _| {},
            slider: Some(Slider { get: |_| HUE_MILLI.load(Relaxed) as f32 / 1000.0, set: |_, v| HUE_MILLI.store((v.clamp(0.0, 1.0) * 1000.0).round() as u32, Relaxed), min: 0.0, step: 0.05, shown: None }),
        });
        add_setting(app, Setting {
            key: "hdr-highlight-expansion",
            group: "Display",
            label: "HDR highlight expansion",
            note: "Spreads the brightest part of the picture (sky, sunlit sand, white walls) into the HDR range. Shadows and mid tones stay. 0 %: the picture exactly as in SDR, only light sources brighter.",
            switches: "",
            get: |_| false,
            set: |_, _| {},
            slider: Some(Slider { get: |_| EXPAND_MILLI.load(Relaxed) as f32 / 1000.0, set: |_, v| EXPAND_MILLI.store((v.clamp(0.0, 1.0) * 1000.0).round() as u32, Relaxed), min: 0.0, step: 0.05, shown: None }),
        });
        add_setting(app, Setting {
            key: "hdr-source-brightness",
            group: "Display",
            label: "HDR light source brightness",
            note: "How bright light that SDR cuts off at white gets: the sun, fire, explosions, neon, lamps.",
            switches: "",
            get: |_| false,
            set: |_, _| {},
            slider: Some(Slider { get: |_| (SOURCE_GAIN_TENTHS.load(Relaxed).clamp(10, 160) - 10) as f32 / 150.0, set: |_, v| SOURCE_GAIN_TENTHS.store(10 + ((v.clamp(0.0, 1.0) * 30.0).round() as u32) * 5, Relaxed), min: 0.0, step: 1.0 / 30.0, shown: Some(|v| format!("x{:.1}", (10 + ((v * 30.0).round() as u32) * 5) as f32 / 10.0)) }),
        });
        add_setting(app, Setting {
            key: "hdr-heatmap",
            group: "Display",
            label: "HDR brightness heatmap",
            note: "Shows how bright each pixel is: grey up to paper white, then cyan, green, yellow and orange, and red at the peak. For calibration.",
            switches: "",
            get: |_| HEATMAP.load(Relaxed),
            set: |_, on| HEATMAP.store(on, Relaxed),
            slider: None,
        });
        app.add_systems(Update, (mark_started, follow_monitor));
        bevy::asset::embedded_asset!(app, "hdr_expand.wesl");
        let Some(render) = app.get_sub_app_mut(RenderApp) else { return };
        render
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Core3d, hdr_expand.after(Core3dSystems::PostProcess).before(bevy::ui_render::render_pass::ui_pass));
    }
}

#[derive(Resource)]
struct HdrPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    id: CachedRenderPipelineId,
    params: bevy::render::render_resource::Buffer,
}

fn init_pipeline(mut commands: Commands, device: Res<RenderDevice>, assets: Res<AssetServer>, fullscreen: Res<FullscreenShader>, cache: Res<PipelineCache>) {
    let layout = BindGroupLayoutDescriptor::new(
        "nvf_hdr_layout",
        &BindGroupLayoutEntries::sequential(ShaderStages::FRAGMENT, (texture_2d(TextureSampleType::Float { filterable: true }), sampler(SamplerBindingType::Filtering), bevy::render::render_resource::binding_types::uniform_buffer_sized(false, None))),
    );
    let sampler = device.create_sampler(&SamplerDescriptor {
        mag_filter: FilterMode::Nearest,
        min_filter: FilterMode::Nearest,
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        ..default()
    });
    let id = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("nvf_hdr_expand".into()),
        layout: vec![layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: assets.load("embedded://nvforever/hdr_expand.wesl"),
            targets: vec![Some(ColorTargetState { format: TextureFormat::Rgba16Float, blend: None, write_mask: ColorWrites::ALL })],
            ..default()
        }),
        ..default()
    });
    let params = device.create_buffer(&bevy::render::render_resource::BufferDescriptor {
        label: Some("nvf_hdr_params"),
        size: 32,
        usage: bevy::render::render_resource::BufferUsages::UNIFORM | bevy::render::render_resource::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    commands.insert_resource(HdrPipeline { layout, sampler, id, params });
}

/// Stretch the highlights of an fp16 main view before the HUD is drawn.
fn hdr_expand(view: ViewQuery<&ViewTarget>, pipeline: Option<Res<HdrPipeline>>, cache: Res<PipelineCache>, device: Res<RenderDevice>, queue: Res<bevy::render::renderer::RenderQueue>, mut ctx: RenderContext) {
    let target = view.into_inner();
    let Some(p) = pipeline else { return };
    if !bevy::render::view::window::NVF_HDR_SURFACE_ACTIVE.load(Relaxed) || target.main_texture_format() != TextureFormat::Rgba16Float {
        return;
    }
    // Peak and paper white from the rows now (live calibration).
    // The pass works in units of the HUD's white (the blit's scale): the picture's white is
    // `scene` of them and its peak `ratio`.
    let hud = bevy::core_pipeline::upscaling::nvf_hdr_paper_white().max(80) as f32;
    let scene = paper_nits().clamp(80, 1000) as f32 / hud;
    let ratio = (peak_nits() as f32 / hud).max(scene);
    let params = [
        ratio,
        HUE_MILLI.load(Relaxed) as f32 / 1000.0,
        scene,
        if HEATMAP.load(Relaxed) { 1.0 } else { 0.0 },
        SOURCE_GAIN_TENTHS.load(Relaxed) as f32 / 10.0,
        EXPAND_MILLI.load(Relaxed) as f32 / 1000.0,
        EXPAND_KNEE,
        0.0,
    ];
    queue.write_buffer(&p.params, 0, bytemuck_bytes(&params));
    let Some(rp) = cache.get_render_pipeline(p.id) else { return };
    let layout = cache.get_bind_group_layout(&p.layout);
    let post = target.post_process_write();
    let bind_group = device.create_bind_group("nvf_hdr_expand", &layout, &BindGroupEntries::sequential((post.source, &p.sampler, p.params.as_entire_binding())));
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("nvf_hdr_expand"),
        color_attachments: &[Some(RenderPassColorAttachment { view: post.destination, depth_slice: None, resolve_target: None, ops: Operations::default() })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(rp);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..3, 0..1);
}

/// The bytes of four floats, little-endian as the GPU reads them.
fn bytemuck_bytes(v: &[f32; 8]) -> &[u8] {
    // SAFETY: f32 has no padding; the slice covers the array exactly.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), 32) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CPU mirror of hdr_expand.wesl's highlight expansion and light-source shoulder (no hue
    /// restore), in units of the picture's white: `peak` over paper white, `amount` the
    /// expansion (0 to 1), `gain` the source gain.
    fn expand(rgb: [f32; 3], peak: f32, amount: f32, gain: f32) -> [f32; 3] {
        let room = peak - 1.0;
        let sdr = rgb.map(|c| c.clamp(0.0, 1.0));
        let m = sdr[0].max(sdr[1]).max(sdr[2]);
        let white = 1.0 + amount * 0.6 * room.max(0.0);
        let mut out = sdr;
        if m > EXPAND_KNEE && white > 1.0 {
            let t = (m - EXPAND_KNEE) / (1.0 - EXPAND_KNEE);
            let a = (white - EXPAND_KNEE) / (1.0 - EXPAND_KNEE) - 1.0;
            let out_m = EXPAND_KNEE + (m - EXPAND_KNEE) * (1.0 + a * t * t);
            out = sdr.map(|c| c * out_m / m);
        }
        let e = [0, 1, 2].map(|i| (rgb[i].max(0.0) - sdr[i]) * gain);
        let em = e[0].max(e[1]).max(e[2]);
        let room2 = peak - white;
        if room2 > 0.0 && em > 0.0 {
            for i in 0..3 {
                out[i] += e[i] * room2 / (em + room2);
            }
        }
        out
    }

    #[test]
    fn sdr_range_unchanged_without_expansion() {
        for r in 0..=20 {
            for g in 0..=20 {
                let c = [r as f32 / 20.0, g as f32 / 20.0, 0.3];
                assert_eq!(expand(c, 3.0, 0.0, 6.0), c);
            }
        }
    }

    #[test]
    fn below_the_knee_unchanged_with_expansion() {
        let c = [EXPAND_KNEE, 0.4, 0.1];
        assert_eq!(expand(c, 3.0, 1.0, 6.0), c);
    }

    #[test]
    fn never_past_the_peak() {
        for peak in [1.5_f32, 3.0, 10.0] {
            for v in [1.0_f32, 2.0, 18.0, 1.0e6] {
                let out = expand([v, v * 0.5, 0.1], peak, 1.0, 16.0);
                assert!(out.iter().all(|&c| c <= peak + 1e-4), "{out:?} past {peak}");
            }
        }
    }

    #[test]
    fn brighter_light_stays_brighter() {
        let mut last = -1.0;
        for i in 0..=400 {
            let v = i as f32 * 0.125;
            let out = expand([v, v, v], 3.08, 0.5, 6.0)[0];
            assert!(out >= last, "{v}: {out} below {last}");
            last = out;
        }
    }

    #[test]
    fn continuous_at_white() {
        let below = expand([1.0, 1.0, 1.0], 3.08, 0.5, 6.0)[0];
        let above = expand([1.0001, 1.0001, 1.0001], 3.08, 0.5, 6.0)[0];
        assert!((above - below).abs() < 0.01);
    }

    #[test]
    fn light_source_keeps_its_colour() {
        // Red 5, green 2.5: both past white by 4 and 1.5, so their light above SDR white
        // keeps that ratio.
        let out = expand([5.0, 2.5, 0.5], 3.0, 0.0, 6.0);
        let ratio = (out[0] - 1.0) / (out[1] - 1.0);
        assert!((ratio - 4.0 / 1.5).abs() < 1e-3, "{ratio}");
    }

    #[test]
    fn peak_slider_auto_then_steps_of_50() {
        set_peak_part(0.0);
        assert_eq!(PEAK_NITS.load(Relaxed), 0);
        assert_eq!(peak_part(), 0.0);
        AUTO_PEAK.store(0, Relaxed);
        assert_eq!(peak_nits(), FALLBACK_PEAK);
        AUTO_PEAK.store(616, Relaxed);
        assert_eq!(peak_nits(), 616);
        assert_eq!(peak_shown(0.0), "Auto (616 nits)");
        set_peak_part(1.0 / 33.0);
        assert_eq!(PEAK_NITS.load(Relaxed), 400);
        set_peak_part(1.0);
        assert_eq!(PEAK_NITS.load(Relaxed), 2000);
        assert_eq!(peak_nits(), 2000);
        for k in 0..=33 {
            set_peak_part(k as f32 / 33.0);
            let n = PEAK_NITS.load(Relaxed);
            set_peak_part(peak_part());
            assert_eq!(PEAK_NITS.load(Relaxed), n, "step {k}");
        }
    }
}
