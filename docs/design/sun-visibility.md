# Sun visibility: one factor on the sun's direct light

Status 2026-11-17: the plumbing (stage C0 of cloud shadows #2670, the base for terrain
shadows #2671) with a debug view and a test producer, and the cloud producer (C1,
`engine/nvforever/src/cloud_shadows.rs`), which fills the cloud slot while Cloud shadows is
switched on (off by default; `NVF_PLAY_DEBUG_CLOUD_SHADOWS=1` for now). Terrain shadows
(T0-T2) fill the terrain slot later. Runs so far: main session run 2026-11-17, fork-notes
(the checkerboard reached terrain, objects, trees and distant hills at Goodsprings 12:00;
no shader or layout errors; an interior run clean). What is not yet shown is listed at the
end.

Like everything in [graphics-enhanced.md](graphics-enhanced.md), this is an engineering
choice, not a fact about the original game.

## 1. The interface

Producers write the main-world resource `SunVisibility` (`engine/nvforever/src/sun_visibility.rs`):

| Field | Type | Meaning |
|---|---|---|
| `cloud_enabled` | `bool` | Cloud shadows is switched on: the views take the cloud variant whether or not `cloud` is set; without `cloud` the cloud is neutral (K = 1) |
| `cloud` | `Option<CloudVisibility { image, uv_scale, uv_offset, plane_height, strength }>` | K: a transmittance texture (red, 1 clear, 0 opaque) on a horizontal plane `plane_height` up, looked up where the ray from the receiver towards the light meets it, at uv = hit.xz × `uv_scale` + `uv_offset` (repeating); K = 1 − `strength` × (1 − texel) |
| `terrain` | `Option<TerrainVisibility { image, uv_scale, uv_offset, fade_band, edge_fade, height_softness }>` | T: a shadow-height map over world xz (#2671); its sampling is a stub returning 1 until stage T2 |
| `debug` | `DebugMode` (`Off`, `Cloud`, `Terrain`, `Combined`) | the debug view |

Axes are Bevy's world axes (y up), units game units. In the shader
(`engine/nvforever/src/sun_visibility.wesl`):

- `nvf_sun_visibility(world_position, direction_to_light, cascade_shadow, view_z) -> f32`
  gives V; `nvf_sun_visibility_parts` gives K, T, Vsolid and V together.
- The resources sit in the mesh view bind group at bindings 39 (uniform
  `NvfSunVisibility`: cloud uv transform, plane height, strength, horizon fade; terrain uv
  transform, fade band, edge fade, height softness; feature mask, debug mode), 40 (cloud
  texture), 41 (terrain map; the white fallback for now) and 42 (a linear, repeating
  sampler) (`engine/vendor/bevy_pbr/src/render/mesh_view_bindings.rs`,
  `NvfSunVisibilityBindings`; engine/vendor/README.md).
- Shader defs: `NVF_SUN_VISIBILITY` while the plumbing is on for the view,
  `NVF_CLOUD_VISIBILITY` while Cloud shadows is switched on (or the test producer sets a
  cloud), `NVF_TERRAIN_VISIBILITY` with a terrain producer.
- Switched on and published are kept apart: the switch alone decides the markers, the
  layout key bit, the pipeline key bits and the shader defs; the publication only fills
  the uniform and the texture. Switched on with nothing published, the uniform carries a
  neutral cloud (strength 0, the plane below every receiver) and the texture binding is the
  white fallback, so K = 1.

## 2. The composition rule

V = K × Vsolid, with Vsolid = min(C, T) nearer than the band `fade_band` (view distance),
T beyond it, and a smooth blend across it; C is the cascade shadow as Bevy computes it
(contact shadows and the mesh's shadow factor included), T the terrain's visibility, K the
clouds'. Without terrain, Vsolid = C exactly: the band does nothing.
[engineering choice, from the plan for #2670/#2671]

Why `min` and not C × T [engineering choice]: near the camera the cascades already contain
the terrain as a caster, so a ridge's shadow is in both C and T. Multiplied, the same
filtered edge would be squared (darker and harder than either). The minimum counts the
darker of the two once. It is an approximation: two scalar masks cannot rebuild the exact
union of two different occluders' soft edges; that would need shared samples or separate
terrain and object shadow maps. Beyond the last cascade C is 1 (Bevy's
`fetch_directional_shadow` returns lit there), so the band fades object shadows out towards
T instead of ending at a line. K multiplies, as clouds are a different medium from the
solid occluders. With sun shadows off C is 1 and Vsolid = T.

Where it acts (`HOOK_EDITS` in sun_visibility.rs, patched into bevy_pbr's
`apply_pbr_lighting` after lights.rs's display-value edits): in the directional light loop,
the outdoor sun's `shadow` becomes V before the light is added, so Bevy's `direct_light`,
the display-value sum `nvf_direct` and the transmitted lobe (`transmitted_shadow`) all take
it. It is outside the `MESH_FLAGS_SHADOW_RECEIVER_BIT` condition, so tree leaves, distant
tree cards and water (not cascade receivers) take it too. Ambient, emissive, point and spot
lights, fog and reflections are left alone.

Only the sun: the `Sun` light (sky.rs) carries the marker `OutdoorSunLight` while outdoors,
extracted as bevy_pbr's `NvfOutdoorSun`, which sets `DIRECTIONAL_LIGHT_FLAGS_NVF_OUTDOOR_SUN_BIT`
in its GPU flags; the hook tests that bit. At night the same light is the moon's, so V
applies to the moonlight too. Indoors the same entity is the cell's directional light and
is unmarked. The first-person arms' shade light and menu views' lights never carry it.

Which views: the world camera (`FlyCam`), the water reflection camera and a scope's lens
camera. While Cloud shadows is switched on they keep their bindings, layout key bit and
shader defs through doors, missing or still-decoding textures and weathers without cover:
the producer clearing its output (it does indoors, cloud_shadows.rs) only makes the cloud
neutral, and indoors the light is unmarked, so nothing darkens. Only switching the feature
on or off changes the views' pipelines. A unit test runs the producer's own door path
(cloud_shadows.rs `cloud_bindings_survive_doors`); that no pipeline is built at a door is
for a run to show (§6).

## 3. Off by default

`SunVisibility` starts empty. While nothing is switched on (no producer, no debug view) no
camera is marked, no view has the bindings, the mesh view layout key bit and the pipeline
key bits are clear, and every statement the hook adds is under `@if(NVF_SUN_VISIBILITY)`,
so the shader a pipeline compiles has the display-value patch's statements (the raw text
differs; compiled-byte identity is not established). No GPU buffer, sampler or image is
made until it is first on (the uniform buffer and sampler are then kept after switching
off).

What the off path still costs [documented: the code; hypothesis: negligible, unmeasured]:

- At start: the shader `sun_visibility.wesl` is loaded, the debug aids are read, and the
  hooked copy of `apply_pbr_lighting` is made as a string (lights.rs, `hook_patched`).
- Every frame: run-condition checks (`resource_changed`, `marking_needed`, the cloud
  producer's switch and change checks), extraction's early return and preparation's empty
  query; in the vendored bevy_pbr, the views' query reads an optional component
  (`mesh.rs`, `mesh_view_bindings.rs`) and `prepare_lights` tests the sun marker per
  directional light (`light.rs`); sky.rs calls the cached cloud-phase helper.
- The pipeline key's packed fields moved up by three bits (`MeshPipelineKey::LAST_FLAG`),
  a different numbering of the same keys, not more of them.

Turning it on or off changes the views' pipeline keys, so every material pipeline of those
views is built again, as any view key change does; it is the only thing that does
(§2, "Which views").

## 4. Test aids

- `NVF_PLAY_DEBUG_SUN_VISIBILITY=cloud|terrain|combined`: draws K, T or V (grey, 1 white)
  in place of the lit colour, before ambient and fog (the fog block is left out under the
  debug view). The image space grade and the eye adaptation still act afterwards
  (`--no-grade` leaves the grade out). Alone it turns the plumbing on, and `combined` then
  shows the cascade shadow C.
- `NVF_PLAY_DEBUG_SUN_VISIBILITY_TEST=checker`: a built-in producer filling `cloud` with a
  64×64 checkerboard of clear and opaque cells of 2,000 units, strength 1, on a plane
  20,000 units up.
- Log: `sun visibility: cloud <on|off> terrain <on|off> debug <mode>` whenever the switches
  change, and `sun visibility: test producer: ...` for the test producer.

## 5. What the later stages owe

- Water's own sun highlight (`engine/nvforever/src/water.wesl`, the `reflected += spec *
  sun_light` line in the `lights.n_directional_lights > 0u` block) reads directional light 0
  and is outside `apply_pbr_lighting`: it must pick the light by the sun bit and multiply by
  `nvf_sun_visibility(pos, L, 1.0, view_z)` under `@if(NVF_SUN_VISIBILITY)` (stage C2).
- The first-person arms' `with_shade_light` (`engine/nvf-assets/src/nif_material.wesl`)
  rebuilds the sun's shadow from two `apply_pbr_lighting` results; its algebra must take V
  into account (stage C2).
- Terrain: T's sampling and the fade band come with T2.

## 6. Not yet proved

Only a run can show: that every pipeline of a marked view compiles with the bindings (the
WGSL of `sun_visibility.wesl` is checked by naga in a unit test, the patched
`pbr_functions` is not); that the layouts match on the reflection and scope views; that
the clouds land on actors, grass, leaves, tree cards and water; that a door, a weather
change or a texture read builds no pipeline; the cost, on and off.
