# Mesh terrain blend, round 1 (#2661)

[design] This is NVForever's aid-only world-space colour blend, selected by
`NVF_PLAY_TEST_MESH_BLEND=24:surface`. Band, selection, resolution and residency rules
are engineering choices, not statements about another renderer. There is no settings row.

[design] `surface`, `weight`, `normal` and `selection` are accepted, with an optional
positive finite band before `:`. A bare variant or empty band uses 24 units. Invalid
values log rejection and leave the aid off. `normal` logs "not yet" and does no work.
See `engine/nvforever/src/mesh_terrain_blend.rs::parse` and `config`.

[design] The supported selection is rigid exterior STAT models, without rigs, skinning,
particles, morphing meshes, loose/jointed bodies or damage-stage nodes. Destructible records, imposters,
external-emittance shapes and interiors are excluded. Only lit opaque/masked materials
without decal, refraction, transmission or special coverage effects receive cloned
variants. SCOL and the other record types remain excluded. Selection code:
`world.rs::spawn_model`, `mesh_terrain_blend.rs::eligible_reference/eligible_material`.
Record access uses existing `fnv-world/src/placed.rs`, `index.rs`, `fields.rs`, and the
DEST-presence convention in `fnv-game/src/data.rs`.

[design] Each supported cell contributes the same prepared heights and four complete
terrain surfaces used by `terrain.rs::spawn_terrain`. Flat copies of its prepared meshes
draw with copies of the same texture handles and folded material values. The ordinary
`terrain.wesl` layer evaluation is shared unchanged; `terrain_surface.wesl` returns its
linear unlit colour before scorch, lighting or fog. A dedicated orthographic GPU view
on render layer 15 bakes 512 by 512 texels per cell into a 1536 by 1536 RGBA16F atlas.
The 3 by 3 region follows the main camera's cell. Colour storage is 18 MiB, plus a
99 by 99 RGBA8 height atlas (39,204 bytes), excluding CPU copies and transient render
targets/readback buffers. See `terrain_surface.rs`.

[design] The height atlas stores the prepared f32 bits losslessly in four UNORM bytes,
so it can use NifSurface's existing filterable bindless texture table without requiring
filterable R32F. The forward fragment reads the four grid vertices with `textureLoad`
and uses the SW/E/NE or SW/NE/N triangle from `terrain.rs::quadrant_mesh`, with mathematical
floor for negative cells. Colour is filtered separately. No averaged heights are introduced.

[design] Publication waits for GPU readback with opaque output in every quadrant of
every included cell. Cell-set, camera-cell, interior and worldspace changes invalidate
the whole generation; stale readbacks are ignored. Source materials are retained per
entity; enabled clones are shared per source handle and generation. Missing cells have
zero influence, with a 128-unit border fade inside adjacent resident cells. The
readiness line reports nonzero eligible-shape and resident-cell counts after publication.

[design] The forward fragment mixes RGB before lighting with
`1 - smoothstep(0, band, max(height_above_ground, 0))`, multiplied by the residency fade.
Noise is zero. Weight mode shows this weight; selection mode tints eligible shapes.
Alpha, discard, vertices, prepass and depth descriptors retain their existing paths
(`nif_material.rs`, `nif_material.wesl`, `nif_surface_bindings.wesl`).

[design] Limits for later rounds:

- Whole cells with overflow layers, missing folded textures, or painted fallback draws
  are omitted rather than baking an incomplete colour. Base-only quadrants are supported.
- No normal/material-response blending, scorch, mip chain, gutters, resolution selector,
  distance fade, temporal history/reactivity integration or visual acceptance result.
- Residency changes rebake the entire bounded atlas. There is no per-frame bake budget,
  partial dirty-cell update, texture/layer invalidation, or streaming performance result.
- Material overrides replacing a handle are respected; in-place source-material edits
  do not refresh already cached clones. Broader selection lifecycle work remains round 4.
- GPU diagnostic zones and performance measurements remain acceptance work.

[hypothesis] The 512-texel surface cache may soften road/rock contact detail. The main
session must inspect fixed-view Off/surface/weight/selection captures before accepting
its quality. No GPU or game validation was performed in this writer round.

[design] Main-session validation targets: nvforever tests `mesh_terrain_blend::tests`,
`terrain_surface::tests`, `terrain::tests`; nvf-assets tests `nif_material::tests`;
smoke `mesh-terrain-blend-goodsprings`. Preserve off-state `terrain-edge-stitch`,
`placed-water-no-reflect`, `tops-13th-floor-hallway`, `tops-suite-light`,
`smoke-blend-bison-steve`, `stealth-boy-refraction`, `graphics-enhanced`,
`nif-bindless-batches`, `pipboy`, `pipboy-screen-fx` and `pipboy-screen-text`.
Compare with the aid
absent, `24:surface`, `24:weight`, `24:selection`; `24:normal` checks only the stub log.
Use the established Primm viewpoint from the main session, with AA off and its normal
AA/upscaler configurations. This document supplies no new Primm viewpoint.
