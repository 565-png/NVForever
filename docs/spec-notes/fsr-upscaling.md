# FSR temporal upscaling inputs

An optional renderer feature of NVForever's own, not a claim about the original game.
Design and measurements: [fsr-upscaling](../design/fsr-upscaling.md).

- [documented] AMD FSR 3.1.4 rebuilds output-resolution colour from jittered
  render-resolution colour, depth and motion vectors. It replaces temporal anti-aliasing
  and runs before tonemapping and the UI. Motion vectors point from the current pixel to
  its position in the previous frame. A camera cut needs a history reset. A texture mip
  bias of log2(render/display) − 1 keeps texture detail at the display's level. Source:
  <https://gpuopen.com/manuals/fidelityfx_sdk/techniques/super-resolution-upscaler/>.
- [documented] AMD's standard scale ratios are Native AA 1, Quality 1.5, Balanced 1.7,
  Performance 2 and Ultra Performance 3, and its jitter helpers use a Halton (2, 3)
  sequence. Source: the same guide, and the MIT shaders in
  `engine/vendor/wgpu-ffx/shaders/include/fsr3upscaler`.
- [observed, in the source] NVForever uses Bevy 0.20.1 and wgpu 30.0.1 (`engine/Cargo.lock`),
  reverse infinite-Z perspective cameras, and its own depth encoding for first-person
  meshes (`nvf-assets/src/first_person_depth.wesl`). Its TAA (`taa.rs`) reprojects with the
  camera alone, which is not enough for FSR's moving objects.
- [observed, in the source] Bevy's prepass motion is current minus previous UV, without the
  jitter, so FSR gets minus the render size as its vector scale. The NIF prepass keeps the
  previous transforms and skinning; grass and tree vertices also need the previous frame's
  wind and billboard turn.
- [engineering choice] Water, refraction and blended particles keep their depth and blend
  behaviour. A mask made from the opaque-only and the final HDR colour lowers FSR's
  accumulation where those surfaces change the image.
- [design] An effective temporal upscaler (FSR or DLSS) replaces native TAA, and TAA leaves
  nothing behind when it is turned off while the game runs: no TAA pass runs on a view that no
  longer carries the TAA marker, and the TAA pass state and history on that view are dropped
  the same frame (#2955; `taa.rs` `drop_stale`). Bevy removes only the extracted marker from
  the render-world entity, which lives on [documented: bevy_extract 0.20.1
  `sync_component.rs`, the removal observer removes `SyncComponent::Target`, by default the
  component itself]. A requested upscaler that does not take effect leaves native TAA on
  (`upscaler::effective_temporal`). TAA turned on again starts its history over.
- [engineering choice] Vulkan only to begin with. Other backends keep native rendering.
  No frame generation.

Sources, licences and the changes to the vendored code: `engine/vendor/README.md`.
