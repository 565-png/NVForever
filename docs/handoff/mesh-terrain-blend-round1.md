# Mesh terrain blend, rounds 1?2 (#2661)

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
on reserved render layer 16 bakes 512 by 512 texels per cell into a 1536 by 1536 RGBA16F atlas.
The 3 by 3 region follows the main camera's cell. Colour storage is 18 MiB, plus a
576 by 99 RGBA8 height/validity atlas (228,096 bytes), excluding CPU copies and transient render
targets. No atlas readback is installed. See `terrain_surface.rs`.

[design] The height atlas stores the f32 bits of the two triangle planes per LAND
square in four UNORM bytes per coefficient. `terrain_surface.rs::height_planes` builds
`[a, east-a, northeast-east]` and `[a, northeast-north, north-a]` from the prepared
heights. Six adjacent texels per square hold these two triples. The shader selects
SW/E/NE or SW/NE/N using `f.y > f.x`, then evaluates the selected plane. All coefficients
use `textureLoad`, byte rounding and bit reconstruction; encoded floats are never
filtered. Constants for resolution, grid, cell size, spacing and height-tile dimensions
come from `nvf-assets/src/nif_surface_blend.rs::blend_shader_defs`; Rust packing uses
the same constants. Mathematical floor handles negative cells.

[design] Each 192 by 33 height tile has 32 rows of coefficients and a metadata row.
The first metadata texel holds the numeric nine-bit neighbour residency mask, produced
by `terrain_surface.rs::residency_mask`. Before filtered colour sampling, the shader
maps all four mip-zero bilinear taps (with clamped atlas edges and reversed atlas Y)
to cells and requires every corresponding bit. This includes missing diagonal cells
in an L-shaped region. The original cardinal 128-unit border fade also remains.

[design] Publication has no pixel download or GPU completion wait. The bake camera uses
`NoIndirectDrawing` and every cloned quadrant uses `NoAutomaticBatching`.
`terrain_surface.rs::install_bake_receipt` wraps Bevy's opaque `DrawMaterial` command
with a receipt reached only after all its commands, including `DrawMesh`, succeed.
An expected entity set names every quadrant of every included cell. The render world
clears receipts each frame; skipped/unprepared draws cannot accumulate across frames.
After `render_system` has submitted commands, `submit_receipts` acknowledges only a
complete set. Its successful view is removed from `SortedCameras` before subsequent
passes can clear the atlas. A shared generation-tagged receipt reaches the main world;
publication waits one additional main frame for queue ordering. Cell-set, camera-cell,
interior and worldspace changes invalidate the generation; stale receipts cannot publish.
See also repository `engine/vendor/bevy_render/src/render_phase/draw.rs` (tuple commands
stop on Skip/Failure), `engine/vendor/bevy_pbr/src/render/mesh.rs::DrawMesh`, and
`engine/vendor/bevy_render/src/renderer/mod.rs::render_system` (submission).

[design] Source materials are retained per entity; enabled clones are shared per source
handle and generation. The readiness line reports nonzero eligible-shape and resident-cell
counts after publication. Publication establishes complete submission, not visual quality.

[design] The forward fragment mixes RGB before lighting with
`1 - smoothstep(0, band, max(height_above_ground, 0))`, multiplied by the residency fade.
Noise is zero. Weight mode shows this weight; selection mode tints eligible shapes.
Alpha, discard, vertices, prepass and depth descriptors retain their existing paths
(`nif_material.rs`, `nif_material.wesl`, `nif_surface_bindings.wesl`).

[design] Round-2 review changes, by file:

- `engine/nvforever/src/terrain_surface.rs`: bake layer 16; production layer-reservation
  uniqueness test covering every production `RenderLayers::layer` target found in
  `engine/nvforever/src` (literal calls in particle/effect tests are fixtures);
  complete-draw submission receipts, freeze and publication delay; production plane
  packing, residency metadata and related tests.
- `engine/nvforever/src/creator_view.rs`: studio moves from 14 to 17, also fixing its
  existing collision with Vigor's 14. VATS keeps 15. Production constants are referenced
  directly by the uniqueness test rather than copied into it.
- `engine/nvf-assets/src/nif_material.rs`: ordinary extension, uniform, key, defaults and
  specialization restored verbatim from `6692d2da4`; GPU-interface regression test.
- `engine/nvf-assets/src/nif_surface_blend.rs` and `lib.rs`: separate
  `ExtendedMaterial<StandardMaterial, NifSurfaceBlend>` with its own 160-byte uniform,
  seven-entry index table and textures. It delegates all ordinary specialization,
  including NIF prepass selection and depth/coverage handling, to `NifSurface`.
  `NIF_SURFACE_BLEND` declares its interface in both stages; `MESH_TERRAIN_BLEND` and
  numeric cache definitions enter only the forward fragment.
- `engine/nvforever/src/mesh_terrain_blend.rs`: registers the separate material plugin
  only when the aid is enabled; swaps eligible entities to that type and restores their
  ordinary handles on invalidation. The separate type was chosen to keep all extra GPU
  storage/bindings out of the ordinary material while sharing shader and prepass code.
- `engine/nvf-assets/src/nif_surface_bindings.wesl`: conditional enabled-only interface,
  plane evaluation and whole bilinear-footprint residency check.
- `engine/nvforever/src/terrain.rs`: tests production planes against positions and
  indexed triangles returned by the actual quadrant mesh builder.
- `engine/nvforever/src/mesh_blend_shader_tests.rs` and `main.rs`: CPU ShaderCache WESL
  composition followed by naga parsing/validation, with blend/interface on/off and
  bindless/non-bindless. A focused binding/blend harness checks composed layout sizes;
  a second test composes complete NIF forward and depth-prepass entry points using
  vendored shader modules. Only the unrelated, non-vendored fullscreen vertex IO type
  imported by Bevy's SSR library is supplied as a small test declaration.
- `tools/smoke/scenarios.toml`: adds the separate material and shader-test paths to the
  existing scenario's selection; expectations and visual acceptance remain unchanged.
- This handoff stays for round 2 as requested. Consolidation into
  `docs/design/graphics-enhanced.md` and removal remain a final-integration task.

[documented: `6692d2da4:engine/nvf-assets/src/nif_material.rs`]
Off-path baseline: ordinary `NifSurfaceUniform` is 128 bytes; extension index range
50..54 has four u32 entries (16 bytes), table binding 100, data binding 101.
Non-bindless bindings are uniform 50, holes texture 51/sampler 52 and FaceGen texture 53.
No binding 54?56 or cache uniform fields belong to this material. The round-2 source
comparison of the complete extension/interface/specialization block against that commit
returned `True`. The new regression test checks generated Rust bindless metadata and
uniform sizes; the composed-shader test checks the WESL layout and binding declarations.
Off does not register the enabled material/cache/render receipt plugin.

[design] Test coverage and limits:

- Production parsing, eligibility and clone preservation remain covered; independent
  test-only weight/height interpolation copies were removed.
- Production height planes are compared with actual mesh triangles over all quadrants,
  using interior barycentric points and a shared edge. Atlas packing/byte reconstruction
  covers every square, both triangles, all coefficients and all nine slots. Production
  cache-origin floor and residency masks include negative coordinates and L-shaped cells.
- Production receipt systems are tested for partial-frame reset, full-set acknowledgement
  and removal of a submitted camera; an empty or incomplete set cannot publish.
- Shader tests compose/validate production code, including height decoding, footprint
  indexing and the only weight formula. They do **not** execute WGSL, sample GPU textures,
  measure weights/colour output, or prove visible axis orientation. Rust tests verify the
  CPU-produced representation, not a second claim of fragment execution. GPU pipeline
  creation, actual render-command success and queue timing still need the main session.
- This writer only type-checks tests. No unit tests, shader validators, builds, game runs,
  screenshots, smoke runs or performance measurements are executed here.

[design] Remaining limitations: incomplete/overflow/fallback terrain cells are omitted;
no normal/material-response blend, scorch, mip chain, gutters, resolution selector,
distance fade, temporal history/reactivity integration or visual acceptance result.
Residency changes rebake the bounded atlas without a per-frame budget or partial updates.
Texture/layer invalidation and in-place source-material edits do not refresh cached clones.
Systems that only query ordinary NIF material components need further lifecycle work for
enabled clones. GPU diagnostic zones and streaming/performance acceptance remain open.

[hypothesis] The 512-texel cache may soften contact detail. Missing-corner rejection can
make a narrow transition where its footprint becomes invalid. Fixed-view captures must
judge both before accepting quality.

[design] Main-session validation: build and run the `nvforever` test filters
`mesh_terrain_blend`, `terrain_surface`, `terrain::tests` and `mesh_blend_shader_tests`,
and the `nvf-assets` filters `nif_material::tests` and `nif_surface_blend::tests`.
Build the game, run smoke `mesh-terrain-blend-goodsprings`, then preserve the Off checks
`terrain-edge-stitch`, `placed-water-no-reflect`, `tops-13th-floor-hallway`,
`tops-suite-light`, `smoke-blend-bison-steve`, `stealth-boy-refraction`,
`graphics-enhanced`, `nif-bindless-batches`, `pipboy`, `pipboy-screen-fx` and
`pipboy-screen-text`. Include overlapping VATS/bake and Vigor/creator camera checks.
Compare identical viewpoints with aid absent, `24:surface`, `24:weight`, `24:selection`;
`24:normal` checks only its stub log. Use the established Primm viewpoint, with AA off
and the main session's usual AA/upscaler configurations. This document supplies no new
Primm viewpoint. Inspect streaming, missing-cell corners and atlas invalidation; measure
GPU cost separately from visible acceptance.

[documented: round-2 validation record in this file] Permitted checks, from `engine`,
with `CARGO_TARGET_DIR=E:/Games/NVForever/engine/target-2661-sol`:

- `cargo check -p nvforever --tests`: exit 0; Finished dev profile, 6.99s.
- `cargo check -p nvf-assets --tests`: exit 0; Finished dev profile, 31.26s.
- `python tools/smoke verify` from worktree root: exit 0;
  `merge check: HEAD is no merge commit (or its parents are not here)`;
  `scenario file: 1517 scenarios, fine`.

[documented: round-2 validation record in this file] Initial checking found the new
extraction annotation/borrow issues and shader-test API/lifetime issues; these were
corrected before the final checks. The first sandboxed check could not open the specified
external target lock; the same authorized command was then permitted with sandbox
escalation. Source changes are confined to this worktree; Cargo used the requested external
cache output. No push was performed.
