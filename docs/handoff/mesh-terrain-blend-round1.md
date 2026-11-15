# Mesh terrain blend, rounds 1-6 (#2661)

[design] This is NVForever's aid-only world-space colour blend, selected by
`NVF_PLAY_TEST_MESH_BLEND=24:surface`. Band, selection, resolution and residency rules
are engineering choices, not statements about another renderer. There is no settings row.

[design] `surface`, `weight`, `normal`, `selection` and `status` are accepted, with an optional
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

[design] Each prepared cell contributes the same LAND heights used by
`terrain.rs::spawn_terrain`, independently of its draw path. Complete folded quadrants
and layer-free fallback bases contribute colour; unsupported quadrants retain heights
and carry an invalid colour bit, without excluding their supported neighbours. Flat copies
of its prepared meshes draw with the same texture handles and folded material values. The ordinary
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
The first metadata texel holds the numeric nine-bit height-neighbour residency mask, produced
by `terrain_surface.rs::residency_mask`. The second metadata texel holds the numeric
four-bit colour mask (SW, SE, NW, NE). Before filtered colour sampling, the shader
maps all four mip-zero bilinear taps (with clamped atlas edges and reversed atlas Y)
to cells/quadrants and requires every corresponding height-residency and colour bit.
This includes missing diagonal cells in an L-shaped region. The original cardinal
128-unit border fade also remains.

[design] Publication has no pixel download or GPU completion wait. The bake camera uses
`NoIndirectDrawing` and every cloned quadrant uses `NoAutomaticBatching`.
`terrain_surface.rs::install_bake_receipt` wraps Bevy's opaque `DrawMaterial` command
with a receipt reached only after all its commands, including `DrawMesh`, succeed.
An expected entity set names every captured colour quadrant. A region containing only
height-only cells publishes on the next main frame without a colour bake or receipt;
its colour is never sampled. The render world clears receipts each frame;
skipped/unprepared draws cannot accumulate across frames.
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
counts after publication. For colour bakes, publication establishes complete submission;
it does not establish visual quality.

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
  and removal of a submitted camera; an empty or incomplete draw set cannot acknowledge a colour bake. Entirely
  height-only regions take the explicit no-colour-bake path described above.
- Shader tests compose/validate production code, including height decoding, footprint
  indexing and the only weight formula. They do **not** execute WGSL, sample GPU textures,
  measure weights/colour output, or prove visible axis orientation. Rust tests verify the
  CPU-produced representation, not a second claim of fragment execution. GPU pipeline
  creation, actual render-command success and queue timing still need the main session.
- This writer only type-checks tests. No unit tests, shader validators, builds, game runs,
  screenshots, smoke runs or performance measurements are executed here.

[design] Remaining limitations: layered overflow and fallback quadrants have invalid
colour; their cells retain full heights and supported quadrants retain colour. There is
no normal/material-response blend, scorch, mip chain, gutters, resolution selector,
distance fade, temporal history/reactivity integration or visual acceptance result.
Residency changes rebake the bounded atlas without a per-frame budget or partial updates.
Texture/layer invalidation does not rebake captured terrain. Source-material edits
refresh clones when their producer republishes the ordinary handle; arbitrary in-place
asset edits without entity republication remain outside this refresh path. GPU visual
and streaming/performance acceptance remain open.

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
Shoot `24:status` first, then compare identical viewpoints with aid absent,
`24:surface`, `24:weight`, `24:selection`;
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

## Round 3: height coverage, diagnostics and material consumers

[documented: main-session runtime report supplied with the round-3 request] At the
reported WastelandNV road view, cell (-17, 0), camera game (-66629, 1481, 8468), heading
196, pitch -38, the main session recorded `ready eligible-shapes=1994 resident-cells=5
generation=6`. Selection tinted the road/pole; weight matched Off and surface showed no
visible change. This writer has not run that view. Whole-cell exclusion was the leading
source diagnosis; these changes remove it but do not establish GPU visual acceptance.

[design] Coverage takes the permitted **per-quadrant validity** alternative, rather than
implementing ordered translucent layer baking this round:

| Near-terrain path | Published height | Published colour |
| --- | --- | --- |
| Complete folded base and all layers | Whole prepared cell | This quadrant |
| Folded base plus overflow layers | Whole prepared cell | Invalid quadrant: `height-only:layered-overflow` |
| Fallback/non-bindless base with no layers | Whole prepared cell | This quadrant: `fallback` |
| Fallback base with layers | Whole prepared cell | Invalid quadrant: `height-only:fallback-layers` |
| Non-bindless base with layers | Whole prepared cell | Invalid quadrant: `height-only:non-bindless-layers` |
| Missing base mesh | Whole prepared cell | Invalid quadrant: `height-only:missing-base-mesh` |

[design] `terrain.rs` now inserts `SurfaceCell` even with zero captured quadrants.
`terrain_surface.rs::encode_surface_cell` always uploads all prepared height planes and
height-neighbour residency, then the independent four-bit colour mask. A bad quadrant
cannot remove the other three or their cell's heights. Every bilinear colour tap must
also name a valid quadrant. Ordinary `surface`/`weight` return the original mesh colour
in invalid colour quadrants; `status` identifies that outcome. A wholly height-only
region publishes without waiting forever for an empty draw receipt. Mixed regions still
wait for all supported colour draws to submit, followed by the existing publication delay.

[design] Diagnostic precedence and colours in `nif_surface_bindings.wesl`:

1. Magenta: fragment cell is outside the atlas's 3x3 slots.
2. Red: the fragment's height coefficient is invalid.
3. Orange: the fragment's own colour quadrant is invalid.
4. Cyan: at least one bilinear tap has missing height residency or invalid colour.
5. Green: success, with brightness `0.2 + 0.8 * weight` so zero-weight success stays visible.

[design] These are RGB values before ordinary NIF lighting. They do not modify alpha,
discard, coverage, depth or prepass behaviour. `mesh_terrain_blend.rs` accepts `24:status`
as variant 4; `normal` remains a stub. The `ready eligible-shapes=... resident-cells=...
generation=...` prefix is unchanged, with `variant=...` appended. Resident-cell count now
counts height coverage, not four-colour-quadrant compatibility.

[design] One `mesh terrain blend: publication ...` line reports generation, origin,
sorted resident grids with q0..q3 paths and a colour mask, all excluded slots with
`not-loaded`, `no-prepared-LAND` or `invalid-prepared-heights`, the camera-eye XY probe's
cell/slot/square/triangle/raw height bytes/CPU lookup status, and colour/height asset IDs.
The eye probe is beneath the camera, not a claim about an arbitrary road fragment. Pair
the publication generation and IDs with `ready`'s generation and selected variant.
The CPU classifier reads the bytes about to be uploaded; it does not inspect GPU bindings.

[design] File-by-file consumer changes:

- `nvf-assets/src/nif_surface_blend.rs`: owns `BlendShape` and `source_material`, shared
  with the gameplay crate; the ordinary NIF GPU interface is unchanged.
- `nvforever/src/mesh_terrain_blend.rs`: republishes clones in PostUpdate, before asset
  events and mesh/material bookkeeping. Ordinary handles published by Update consumers
  become the new source; an enabled source republished under the same handle also evicts
  its cached clone before clones are shared again. Cache invalidation restores that source.
- `nvforever/src/fade.rs`: includes the blend type in the cell-fade filter. Levels remain
  on the cell and `VisibilityRange` remains on each entity.
- `nvforever/src/draw_distance.rs`: includes both active material types in new/changed/all
  queries, preserves a progressing fade, applies resting ranges and slider updates, and
  marks the active handle changed when adding the first range requires a new pipeline.
- `nvforever/src/door_fade.rs`: supports both material types and marks the active one
  changed on its first range as well.
- `nvf-play/src/fade.rs`: scripted reference fades find an enabled shape's source;
  their existing entity `Fading`/`FadeHidden` state and alpha-copy progression continue.
  Alpha-blended copies use the ordinary type while fading and can rejoin blending afterwards.
- `nvforever/src/texture_swap.rs`: resolves through `BlendShape.original`, publishes the
  swapped ordinary source for clone recreation, and retries retained script swaps when
  reference/material/blend components arrive. The processed generation cannot skip a
  shape merely because it currently draws with the blend type.
- `nvforever/src/effect_shaders.rs`: reference overlays, holes/restoration and alpha
  changes resolve either type's source. Special-coverage/alpha copies still fail ordinary
  blend eligibility; the clone system restores their source and re-enables only when eligible.
- `nvforever/src/shadow_thin.rs`: keeps enabled meshes in reference bounds and per-mesh
  shadow decisions, resolving opacity from their source material.
- `nvforever/src/nif_mirror.rs`: scripted NIF block material edits resolve enabled sources;
  republication refreshes a clone even when an owned source was edited in place.
- `nvforever/src/terrain_surface.rs`: atlas initialization uses `as_chunks_mut::<4>()`;
  no constant-size `chunks_exact_mut(4)` remains in this feature's new code.
- `tools/smoke/scenarios.toml`: selects the changed consumers, requires the publication
  diagnostic line, and gives status-first manual capture guidance. Existing `ready`,
  screenshot and shader-error expectations are retained.

[documented: source search of `MeshMaterial3d<NifMaterial>` in `nvforever/src` and
`nvf-play/src`, round 3] Other producers are actor/rig-only (`actor_materials`, actor
stealth fields), player-only (`stealth_field`, `pipboy_arm`, Vigor), card/interface-only
(`blackjack_view`), script-drawn emittance-only (`emittance`), or inspection paths
(`refraction`, VATS). These do not receive the rigid exterior STAT `BlendShape` marker;
external-emittance, actor, first-person and interface meshes remain excluded. No new
material consumer was inferred from an asset pathname.

[design] New regression coverage, **type-checked only**:

- Production encoding retains every square/triangle's heights in partial and wholly
  height-only cells, with independent colour masks and supported-neighbour success.
- Production CPU publication probes classify all five statuses, including a missing
  diagonal and a bilinear tap crossing an invalid quadrant; log formatting checks grids,
  paths, reasons, probe bytes/status and both asset-ID fields.
- ECS publication followed by a scripted texture swap uses the production swap system
  and synthetic BC1 DDS loader, recreates the enabled clone, retains its entity range,
  survives the processed generation, refreshes an edited source republished under the
  same handle, and restores the swapped source when publication is invalidated.
- ECS scripted fade starts from an enabled mesh, progresses through its ordinary alpha
  copy and restores blending at completion; draw-distance coverage keeps an ongoing fade
  then applies range/slider changes to the blend type. Cell-fade filter includes that type.
- Existing production WESL-composition tests include the updated status/validity code
  when the main session executes them. Cargo check does not execute those validators.

[documented: round-3 command outputs] Final permitted checks from `engine`, all with
`CARGO_TARGET_DIR=E:/Games/NVForever/engine/target-2661-sol`:

- `cargo check -p nvforever --tests`: exit 0; Finished dev profile, 6.95s.
- `cargo check -p nvf-assets --tests`: exit 0; Finished dev profile, 3.57s.
- `cargo check -p nvf-play --tests`: exit 0; Finished dev profile, 1m 40s.
- `python tools/smoke verify` from the worktree root: exit 0;
  `merge check: HEAD is no merge commit (or its parents are not here)`;
  `scenario file: 1517 scenarios, fine`.

[documented: round-3 command outputs] The initial sandboxed Cargo check could not open
the specified external cache lock; the same authorized command succeeded with sandbox
escalation. An intermediate check caught test assertions requiring `Debug` on Bevy's
`VisibilityRange`; they were corrected. No cargo build/test/clippy, shader validation,
game run, smoke run or game-data access was performed. No push was performed.

[design] Main session: capture **`24:status` first** at the supplied WastelandNV road
view, then identical Off / `24:selection` / `24:weight` / `24:surface` views. Preserve the
publication line and `ready` line together. Orange confirms a remaining unsupported
quadrant rather than missing cell heights; magenta distinguishes visible fragments
outside the region; red despite valid CPU probe data calls for GPU binding investigation.
Green brightness establishes the shader's lookup/weight classification only after a
real run. Repeat with non-bindless fallback, overflow layers, streaming, a mid-fade
publication, Objects slider changes and a texture swap while enabled. Main-session test
filters should also include `draw_distance`, `fade`, `door_fade`, `effect_shaders` and
`nif_mirror` in addition to the earlier filters. Existing Off/AA/upscaler checks above
remain required. Unsupported colour quadrants and GPU/performance acceptance stay open.

## Round 4: whole layer stacks, ordering, shadows and failing tests

[documented: main-session runtime report supplied with the round-4 request] At the
round-3 WastelandNV road view, `24:status` drew the road orange (invalid colour
quadrant); the publication line gave `(-17,-1)` colour-mask=0x5 and `(-17,0)` 0x4 with
`height-only:layered-overflow` quadrants, the others folded, `resident-cells=9`, no red.
This writer has not run that view.

[design] Colour now covers each quadrant's whole near-terrain stack, superseding the
round-3 table above. `terrain.rs::spawn_terrain_with` (called by `spawn_terrain` with the
fold decision and `config().is_some()`) collects, per quadrant and in drawing order, the
base draw (folded `TerrainMaterial`, or the fallback/non-bindless base's StandardMaterial)
and then every blended layer draw that the near terrain actually spawns, with that
layer's own StandardMaterial: `AlphaMode::Blend`, texture, `sort_first` depth bias.
Layers the near terrain skips (no texture, no painted triangles) are skipped too.
`terrain_surface.rs::SurfaceCell::capture` flattens each copy and marks it
`surface_bake`; copies after the first are `bake_layer`. No blend arithmetic is
re-implemented in Rust: the GPU's alpha blend state for the material's alpha mode
composites the layer copies, ordered by the same depth bias as the near layer draws
(the bake's orthographic copies all have the same view distance).

| Near-terrain path | Published colour | Label |
| --- | --- | --- |
| Folded base, no further layers | This quadrant | `folded` |
| Folded base plus n blended (overflow) layers | This quadrant | `folded+n-layers` |
| Fallback/non-bindless base, no layers | This quadrant | `fallback` |
| Fallback/non-bindless base plus n blended layers | This quadrant | `fallback+n-layers` |
| No base mesh (cannot occur for prepared land with heights) | Invalid | `height-only:missing-base-mesh` |

[design] Shader: `terrain.wesl` keeps one fragment. With `SURFACE_BAKE_LAYER` it saves
the vertex alpha (the layer opacity) before the folded path clears it, multiplies the
material/texture alpha from `pbr_input_from_standard_material`, and
`terrain_surface.wesl::surface_bake(colour, alpha)` returns unlit colour with that alpha;
base copies return alpha 1. Near layer draws (`NifSurface` defaults: no falloff, cut 0,
soft-fade distance 0, so factor 1) blend with alpha = material x texture x vertex alpha, the
same factor [design: nif_material.wesl `nif_fragment`, `nif_material.rs` defaults].
`TerrainLayersKey` carries `surface_bake` and `bake_layer`; bake pipelines also take the
sort-only part out of the rasteriser depth bias, as `NifSurface::specialize` does with
`raster_bias`. Ordinary terrain keys are both false, so ordinary pipelines are unchanged.

[design] Receipts: `BakeReceipt` is generic over the phase and wraps `DrawMaterial` in
both `Opaque3d` (base copies) and `Transparent3d` (layer copies). The expected entity
set holds every copy, so the atlas publishes only after every quadrant's whole stack
has drawn in one frame; per-quadrant validity follows from that single publication.

[design] Other round-4 changes, by file:

- `nvforever/src/mesh_terrain_blend.rs`: `conversion_order` places the conversion
  after `nif_mirror::MirrorSystems` (scripted NIF edits, their commands applied) and
  after `nvf_assets::nif_material::SeeThroughShadowSystems` (so a fade's restored
  ordinary material clears its `SeeThroughNoShadow`/`NotShadowCaster` before it is
  swapped for the blend material), and before asset events and mesh change marking.
  Tests use the same ordering function.
- `nvforever/src/nif_mirror.rs`: `mirror` is in the new `MirrorSystems` set.
- `nvf-assets/src/nif_material.rs`: `see_through_casts_no_shadow` is in the new public
  `SeeThroughShadowSystems` set; `see_through_shadow_system()` exposes it, in its set,
  for other crates' tests. Its behaviour is unchanged.
- `nvforever/src/texture_swap.rs`: the extra rescan triggers apply only to entities with
  `BlendShape` and `RefVisual` (`NewSwapTargets`); with the aid absent no entity has
  `BlendShape`, so only main's `Added<RefVisual>` can fire. Type aliases, not an allow,
  answer `clippy::type_complexity` there.
- `nvforever/src/outfits.rs`: the test helper `placed_ref` is `pub(crate)`.
- `tools/smoke/scenarios.toml`: the Goodsprings scenario also requires that no
  publication line names a `height-only:` quadrant (stricter, nothing loosened).

[design] Failing tests reported on ec7dbd422:

- `terrain_surface::tests::receipt_requires_all_draws_in_one_frame`: entities come from
  `World::spawn_empty`, not `Entity::from_bits`.
- `mesh_blend_shader_tests::composed_production_blend_module_validates_with_key_on_and_off`:
  the synthetic bindless stub has `enable wgpu_binding_array;` under `@if(BINDLESS)`, as
  the vendored `bindless.wesl` does.
- `mesh_terrain_blend::tests::published_shape_joins_script_fade_and_returns_to_blending_afterwards`:
  the test was wrong. `nvf_play/src/fade.rs::start` skips a fade out of a reference that
  is not going out, and an unknown reference is present (`SimState::present`). The test
  now uses a placed reference that starts disabled (`FLAG_INITIALLY_DISABLED`) and asserts
  it is not present; its fade, alpha and end assertions are unchanged.

[design] New tests, type-checked only:

- `terrain_surface::tests::spawned_layered_quadrants_publish_whole_heights_full_colour_and_every_draw`:
  synthetic land (7, 2, 0 layers and a quadrant without a base record) through
  `prepare_land`, `spawn_terrain_with` (folding on and off) and `update_cache`: paths,
  colour mask 0xf, one bake copy per near `LandMesh` draw, layer copies blended and
  flat, every square's height in the uploaded atlas, CPU lookup success in all four
  quadrants, and the `BakeView` expected set equal to the near draw count.
- `mesh_terrain_blend::tests::finished_fade_in_restores_shadows_before_blending_again`:
  production fade and shadow systems; markers present mid-fade, both removed and the
  blend material back after completion.
- `mesh_terrain_blend::tests::scripted_nif_edit_in_postupdate_is_converted_in_the_same_frame`:
  an edit system in `MirrorSystems` republishes an edited source by command; after one
  frame only the refreshed blend material remains.
- `mesh_blend_shader_tests::terrain_surface_bake_composes_for_base_and_layer_copies`:
  `terrain.wesl` composed and naga-validated for ordinary, bake-base and bake-layer
  variants, bindless and not.

[hypothesis] Remaining risks for the main session's runs: the non-bindless bake uses
`TerrainMaterial` (as round 3's fallback capture did), whose non-bindless layout has 13
extension textures; a GPU without bindless that cannot bind them all would fail that
bake pipeline and never publish. The bake's layer order relies on equal view distances
plus the layers' depth bias; it is checked by code reading, not by a captured frame.

[documented: round-4 command outputs] Permitted checks, from `engine` with
`CARGO_TARGET_DIR=E:/Games/NVForever/engine/target-2661-opus`:
`cargo check -p nvforever --tests` exit 0; `cargo check -p nvf-assets --tests` exit 0.
`python tools/smoke verify` from the worktree root: exit 0, `scenario file: 1517
scenarios, fine`. No cargo build/test/clippy, game run, smoke run or game-data access.

[design] Main session: run the `nvforever` filters `terrain_surface`, `terrain::tests`,
`mesh_terrain_blend`, `mesh_blend_shader_tests`, `texture_swap`, `nif_mirror`, `fade`,
`draw_distance`, `door_fade`, `effect_shaders`, `shadow_thin`, and the `nvf-assets` filters
`nif_material::tests`, `nif_surface_blend::tests`; engine clippy. Then the
`mesh-terrain-blend-goodsprings` scenario and the round-3 road view with `24:status`
first: every resident cell should report `colour-mask=0xf`.

## Round 5: contact weight, shading blend, resolution and a small capture material

Round 5 supersedes the earlier statements about the default band (24), the fixed 512
resolution, colour-only blending, the `normal` stub and the non-bindless bake material.

[observed: main-session report on the round-4 exe, road mesh on the landscape in
WastelandNV cell (-17,0)] `24:status` was all green with `colour-mask=0xf` in all 9 cells.
`24:weight`: the shoulder that cuts into the ground had w=1; the flat road top had
w of about 0.3-0.5, about 10-15 units above the terrain. `24:surface` mean sRGB, off then
on: road top (99,95,73) to (100,95,73); shoulder (99,89,67) to (111,99,73); ground beside
it (141,127,96). The shoulder kept the mesh's lighting and normal-map creases, and 8 units
per texel looked blurred. The round-4 engine tests passed.

[design] The plan this round implements is the reviewer's (fidelity plan for round 5):
weight, shading and detail are treated as three separate defects.

### Aid value

[design] `NVF_PLAY_TEST_MESH_BLEND=[band:]view[:shape[:resolution[:shading]]]`
(`mesh_terrain_blend.rs::parse`):

| Part | Values | Default |
| --- | --- | --- |
| band | positive finite number | 8 |
| view | `surface`, `weight`, `normal`, `selection`, `status` | (required) |
| shape | `height`, `penetration`, `hybrid` | `hybrid` |
| resolution | `512`, `1024`, `2048` (colour texels per cell side) | `1024` |
| shading | `albedo`, `full` | `full` |

The first part is the view when it names one, otherwise the band (empty: the default).
An empty later part takes its default; more parts, an unknown word or another resolution
is rejected in the log and leaves the aid off. Old short forms keep their meaning with the
new defaults for what they omit: `24:status` is band 24, hybrid, 1024, full. The start
line is `mesh terrain blend: start band=... variant=... resolution=... noise=0 shape=...
shading=...`; the ready line appends `shape=... resolution=... shading=...` after its
`ready eligible-shapes=... resident-cells=... generation=... variant=...` prefix, with the
published atlas's resolution. The defaults are NVForever's choices.

### Contact weight

[design] With `d` the fragment's game height above the selected ground triangle
(negative below it), `B` the band, `F` the existing residency/edge fade, `c` the cosine
between the interpolated mesh normal before normal mapping (`pbr_input.world_normal`) and
the ground triangle's geometric normal:

```text
H = 1 - smoothstep(0, B, max(d, 0))
P = 1 - smoothstep(-2, 0, d)
G = 1 - smoothstep(cos 35 deg, cos 15 deg, c)
height:      w = F * H
penetration: w = F * P
hybrid:      w = F * H * max(G, P)
```

The ground normal is `normalize(-dx / 128, -dy / 128, 1)` in game space from the selected
triangle's plane coefficients (`terrain_surface.rs::height_planes`), and `(x, z, -y)` in
Bevy space (`world.rs::game_to_bevy`'s rotation). The 15 and 35 degree limits and the 2-unit
penetration depth are NVForever's initial tuning choices. The CPU statement is
`nvf-assets/src/nif_surface_blend.rs::contact_weight` (also `terrain_game_normal`,
`game_to_world_normal`, `blend_normal`); the shader's `mesh_terrain_weight`,
`terrain_game_normal`, `game_to_world_normal` and `mesh_terrain_blend_normal` in
`nif_surface_bindings.wesl` transcribe them, and the numbers reach the shader as shader
definitions from the same constants (`blend_shader_defs`). The unit tests run the Rust
statements; the WGSL is composed and validated, not executed.

[design] Expected effect, by the formula, not by a run: a road top 10-15 units above the
ground gets no weight at band 8 in any shape; a top parallel to the ground at or above it
gets none in `hybrid`, coplanar included; a face turned 35 degrees or more gets the full
contact weight at the ground, fading over 8 units; anything 2 units or more below the
ground gets full weight in `hybrid` and `penetration`. This is a contact heuristic, not
detection of mesh boundaries.

### Shading

[design] `nif_material.wesl`, forward pass only, after the existing coverage tests and
normal mapping, before lighting (one `mesh_terrain_lookup` returns status, weight, cached
colour and the ground normal). With `w > 0` and a successful lookup:

```text
base_color.rgb = mix(mesh, cached colour, w)          (albedo and full)
N = mesh_terrain_blend_normal(N, ground normal, w)    (full only, below)
perceptual_roughness -> 0.95, reflectance -> 0.2, metallic -> 0, emissive * (1 - w),
clearcoat * (1 - w), anisotropy strength * (1 - w)
```

The targets 0.95 and 0.2 are the landscape's own values, `terrain.rs::surface`, which now
reads them from `nif_surface_blend.rs` (`LAND_PERCEPTUAL_ROUGHNESS`, `LAND_REFLECTANCE`;
unchanged values). Roughness, reflectance and metallic are the values after the mesh's
textures, so its specular and roughness maps fade out at w = 1. The normal mix falls back
to the ground normal where it cancels out; w <= 0 leaves every input exactly as it was.
Alpha, discard, coverage, positions, depth, the geometric `world_normal` (shadow bias),
shadows and occlusion are untouched. Geometric triangle normals, not the landscape's
LAND normals and normal maps, are the target this round [design]: creases and slope
darkening should go, pixel-identical ground lighting is not claimed. A normal cache is
deferred (it would need the terrain's tangent frame and layer normals, another binding,
about 36 MiB for a 3x3 RGBA8 atlas at 1024).

### Views

[design] `weight`, `normal` and `status` are drawn as unlit colour without fog: the
material is switched to unlit with fog off and its emission cleared, without an early
return, so the lighting code keeps uniform control flow. `weight` is blue (0) to red (1)
for the selected shape, the mesh's own colour where the lookup fails. `normal` is the
ground triangle's game-space normal as `0.5 + 0.5 n` (flat ground (0.5, 0.5, 1), east-
facing slopes redder, north-facing greener), black without a valid height. `status`
keeps round 3's colours and precedence. `selection` stays a lit green tint. The
publication line's probe now ends `probe-normal=(x, y, z)` (or `none`), the normal the
`normal` view should show beneath the camera, and the line ends `resolution=...`.

### Resolution

[design] The atlas is `3 x resolution` texels square, RGBA16F, one mip:

| Resolution | Units per texel | Colour storage |
| --- | ---: | ---: |
| 512 | 8 | 18 MiB |
| 1024 (default) | 4 | 72 MiB |
| 2048 | 2 | 288 MiB |

CPU copies, render targets and transient generations are not counted. The shader reads
the resolution from the bound atlas (`textureDimensions`); `MESH_TERRAIN_RESOLUTION` is
gone. `SurfaceCache::resolution` sizes the atlas and feeds the CPU footprint probe
(`lookup_status(..., resolution)`). At startup `check_resolution` compares `3 x resolution`
with the device's `max_texture_dimension_2d`: supported, it logs `mesh terrain blend:
colour atlas resolution=... atlas=WxW rgba16f colour-mib=... max-texture-dimension-2d=...`;
unsupported, it logs `rejected resolution=...` and nothing is baked or sampled. The mip-0
footprint validation is unchanged. RGBA8 sRGB storage and direct dominant-layer sampling
are deferred, as the plan says.

### Single-texture capture (round-4 P2)

[design] `terrain_surface.rs::SurfaceCaptureMaterial` is a plain `Material` (not derived
from StandardMaterial or TerrainMaterial) with one uniform (linear material colour, UV
transform, texture-present flag, layer flag), one diffuse texture and one sampler; its
key carries the layer flag and the rasteriser part of the source's depth bias
(`raster_bias`), and `Material::depth_bias` returns the source's whole bias, so layer
copies sort as the near layers do. Shader `terrain_capture.wesl`: colour = material x
vertex x diffuse (a missing texture counts as white, so the source's fallback grey stays);
a base writes alpha 1, a layer `material x vertex x diffuse` alpha, composited by the
blend state of `AlphaMode::Blend` without depth writes. It is registered by
`SurfaceBakePlugin` (aid on only) with prepass and shadows off. `terrain.rs::
spawn_terrain_with` now records `CaptureDraw::Folded` only for folded bases (which exist
only where `TerrainMaterials::folds` found bindless textures, or never under
`NVF_PLAY_TEST_TERRAIN_LAYERS=blended`) and `CaptureDraw::Single` for every other base and
every layer, layers after a folded base included. TerrainLayers keeps only
`surface_bake` (`SURFACE_BAKE_LAYER` and the layer raster-bias branch are gone).
Receipts, the expected-entity set, both phases and the delayed publication are unchanged.

[design] The blend material's own layout: `NifSurfaceBlend` now binds exactly the
ordinary `NifSurface` slots (uniform 50, texture 51, sampler 52, texture 53, index table
50..54); its colour atlas, sampler and height atlas take the places of the ordinary holes
texture, holes sampler and FaceGen offset, which a blend clone never has (eligibility
requires no holes and no FaceGen; the blend key's `facegen` is false; in the blend
variant the shader's holes/FaceGen readers return constants and are not reached). The
uniform stays 160 bytes. So the blend pipeline binds no more textures per stage than the
ordinary NIF pipeline it replaces: the view and mesh groups are the same mesh pipeline's,
the material groups have equal texture counts. The ordinary NifSurface interface is
untouched (`nif_material.rs::tests::ordinary_surface_gpu_interface_matches_main`).

### Tests, type-checked only

- `nvf-assets` `nif_surface_blend::tests`: weight at heights -2/0/4/8/12 for every shape,
  parallel, 25-degree and upright normals, the 15/35-degree limits, road-top heights,
  clamping; normals of both triangles from the plane coefficients, game-to-Bevy mapping;
  normal blending exact at w <= 0 and safe for opposed or zero normals; shared shader
  constants (no resolution definition); blend layout equal to the ordinary slots and
  texture count (bindless descriptors and the struct's attributes).
- `nvforever` `terrain_surface::tests`: every probe classification at all three
  resolutions; a footprint boundary that rejects at 512 and passes at 1024/2048, at positive
  and negative cells; one-texel cell edges and the missing quadrant at every resolution;
  resolution limits and storage; the probe normal for both triangles, its Bevy rotation and
  its `none` cases; publication line with `resolution=` and `probe-normal=`; the spawned
  layered-quadrant regression now also checks that folding off yields no TerrainMaterial
  copy, layer copies blend with rising sort bias after their base and the right raster
  bias, base copies are opaque, each bake entity's material type, and the 3072-texel
  default atlas; a rejected resolution bakes and publishes nothing.
- `nvforever` `mesh_terrain_blend::tests`: old short forms, every comparison-sheet value,
  band-less and empty parts, rejections; start and ready lines with their prefixes; the
  material controls.
- `nvforever` `mesh_blend_shader_tests`: the binding module composes and validates in all
  four bindless/blend variants with textures only in slots 51 and 53; the complete NIF
  forward (with and without `SOFT_EDGES`) and prepass compose, and every texture the blend
  shader uses is a binding of the ordinary pipeline's layout; the folded bake composes;
  `terrain_capture.wesl` composes with and without colours/coordinates and uses one texture;
  `material_groups_fit_a_16_texture_stage_without_bindless`: blend group = ordinary NIF
  group (<= 16), the capture's group is one texture, TerrainMaterial's exceeds 16 (why it is
  folded-only).
- Not available in this round: a constrained-device GPU test. Unit tests here have no wgpu
  device (wgpu's noop backend feature is off for nvforever, and CI has no GPU), so pipeline
  creation itself is not exercised; the composed shaders show only what each shader uses.

[documented: round-5 command outputs] Permitted checks, from `engine` with
`CARGO_TARGET_DIR=E:/Games/NVForever/engine/target-2661-opus`: `cargo check -p nvf-assets
--tests` exit 0; `cargo check -p nvforever --tests` exit 0; `cargo check -p nvforever`
exit 0, all without warnings. `python tools/smoke verify` from the worktree root: exit 0,
`scenario file: 1518 scenarios, fine`. No cargo build/test/clippy, game run, smoke run or
game-data access.

### Smoke

[design] `mesh-terrain-blend-goodsprings` keeps `24:surface`; its start expectation now
reads `resolution=1024 noise=0 shape=hybrid shading=full` (1024 is the new default; the
`start band=24 variant=surface` prefix is unchanged), and it also requires the `colour
atlas resolution=1024 atlas=3072x3072 rgba16f colour-mib=72` line. Its other expectations
are unchanged; `terrain_capture.wesl` joins its paths. New `mesh-terrain-blend-options`
runs `8:status:penetration:2048:albedo` and requires the resolved start line, the 2048
atlas line, a ready line with `variant=status shape=penetration resolution=2048
shading=albedo`, a publication line ending `resolution=2048`, and no shader errors.

### Comparison sheet for the main session

[design] Identical camera, time and exposure; the round-3/4 WastelandNV road view (cell
(-17, 0), camera game (-66629, 1481, 8468), heading 196, pitch -38) first:

| Frame | `NVF_PLAY_TEST_MESH_BLEND` | Isolates |
| --- | --- | --- |
| Off | unset | the original mesh |
| Round-4 reference | `24:surface:height:512:albedo` | round 4's behaviour |
| Narrow band | `8:surface:height:512:albedo` | the band alone |
| Contact shape | `8:surface:hybrid:512:albedo` | orientation and penetration |
| Lighting match | `8:surface:hybrid:512:full` | the shading blend |
| Recommended | `8:surface:hybrid:1024:full` | the sharper cache |

Optional: `8:surface:hybrid:2048:full`, `8:surface:penetration:1024:full`. Diagnostics:
`8:weight:hybrid:1024:full`, `8:status:hybrid:1024:full`, `8:selection:hybrid:1024:full`,
`8:normal:hybrid:1024:full` (compare its colour beneath the camera with the publication
line's `probe-normal`). Then repeat Recommended and Off at a rock foot and at a cliff base:
the road alone cannot judge those cases. Round 4's `24:surface` frames equal the round-4
reference value only in band, shape and resolution; in round 5 `24:surface` means
`24:surface:hybrid:1024:full`.

[hypothesis] Risks to look for: shallow rock feet may blend too little under the
orientation gate; authored mesh normals can misclassify edges; triangle normals can show
faceting and lack the LAND normal maps; lighting still differs through shadows, occlusion
and the separately lit landscape layer draws; 1024 may still look soft, 2048 costs memory
and capture time and may alias; buried visible surfaces still take the landscape's look.

[design] Main session: run the `nvforever` filters `terrain_surface`, `terrain::tests`,
`mesh_terrain_blend`, `mesh_blend_shader_tests`, `texture_swap`, `nif_mirror`, `fade`,
`draw_distance`, `door_fade`, `effect_shaders`, `shadow_thin`, and the `nvf-assets`
filters `nif_material::tests`, `nif_surface_blend::tests`; engine clippy; smoke
`mesh-terrain-blend-goodsprings` and `mesh-terrain-blend-options` plus round 2's Off
checks; then the comparison sheet. If the game starts with `WGPU_SETTINGS_PRIO=webgpu`
(no optional features, 16 sampled textures per stage), a run of `8:status:hybrid:1024:full`
there is the constrained-device capture check the unit tests cannot make.

## Round 6: hybrid v2 band widths, the STAT census and wider material admission

Round 6 supersedes round 5's statements that `hybrid` is the orientation-gated formula,
that the enabled uniform is 160 bytes, that Blend, depth-biased, decal and alpha-cut
materials are excluded, and that the ready line is printed once per generation with
nonzero eligible shapes.

[design] The plan this round implements is the reviewer's (round 6 plan): steps 1
(weight shape and widths), 2 (census; the colour-coded view was not built, below), 3
(admissions) and 5 (tests and this section).

### Aid value

[design] `NVF_PLAY_TEST_MESH_BLEND=[band:]view[:shape[:resolution[:shading[:flat,steep]]]]`
(`mesh_terrain_blend.rs::parse`). The shape part also accepts `hybrid-v1` (round 5's
hybrid, kept for regression comparisons). The sixth part is a `flat,steep` pair of
game-unit widths for `hybrid` only; omitted, they are `band / 16` and `6 band` (0.5 and
48 at the default band of 8). Both must be finite and positive with
`flat <= band <= steep`; a malformed pair, an empty sixth part, a pair with another shape
or a derived width that is not finite (a band of 3e38 with `hybrid`) is rejected in the
log and leaves the aid off. Every value accepted in round 5 is still accepted, with the
same meaning for `height`, `penetration` and every part but the meaning of `hybrid`. The
defaults are NVForever's choices.

Start and ready lines keep every round 5 field and prefix, then append
` flat=... steep=... weight-version=2` for `hybrid`, or ` weight-version=1` for the
shapes round 6 left unchanged (`height`, `penetration`, `hybrid-v1`), e.g.
`mesh terrain blend: start band=8 variant=surface resolution=1024 noise=0 shape=hybrid
shading=full flat=0.5 steep=48 weight-version=2`.

### Contact weight (hybrid v2)

[design] With `d`, `F` and the unperturbed mesh normal as in round 5, the ground
triangle's normal `n`, and `S` WGSL's `smoothstep`:

```text
c       = clamp(abs(dot(normalize(mesh normal), n)), 0, 1)
shallow = 1 - S(cos 5 deg, 1, c)
steep   = 1 - S(cos 75 deg, cos 35 deg, c)
B       = flat + (band - flat) * shallow + (steep_width - band) * steep
H       = 1 - S(0, B, max(d, 0));  P = 1 - S(-2, 0, d)
hybrid (v2): w = F * max(H, P)
```

Surfaces within 5 degrees of the ground get the flat width (0.5), shoulders turned 5-35
degrees round 5's successful 8-unit band, faces turned 75 degrees or more 48 units; the
width ramps continuously between, so there is no angular on/off gate, and `abs` treats
opposite parallel normals alike. The degenerate-normal fallback (cosine 1, the flat width)
is retained. Below the ground H = 1, so P only matters to the `penetration` shape, whose
ramp is unchanged. By the formula (not a run): a parallel asphalt top 10-15 units above
the ground gets no weight. The 5, 35 and 75 degree limits and the width defaults are
NVForever's tuning choices. Noise stays off (`noise=0`); the plan's next step if a rock
contour shows is a deterministic world-space value noise on the steep width only.

[design] CPU statement: `nvf-assets/src/nif_surface_blend.rs::{v2_band, contact_weight,
ContactWidths}` (`ContactShape::Hybrid` = 2, `HybridV1` = 3 appended, numbers stable);
shader: `mesh_terrain_band` and `mesh_terrain_weight` in `nif_surface_bindings.wesl`, the
cosines through `blend_shader_defs` (`MESH_TERRAIN_V2_COS_PARALLEL_E6`,
`MESH_TERRAIN_V2_COS_STEEP_E6`, `MESH_TERRAIN_COS_FULL_E6`). The widths reach the shader
in one vec4 appended to the enabled-only uniform (`mesh_terrain_widths`: x flat, y steep):
the enabled uniform is now 176 bytes, the ordinary NIF uniform stays 128, and the texture
bindings stay 51/52/53. The zero-weight branch is unchanged: `w <= 0` leaves every
shading input as it was. Terrain-triangle normal discontinuities and cache-validity
boundaries remain discontinuities of the weight.

### Census before filtering

[design] `world.rs::spawn_model` gives every spawned STAT shape a
`mesh_terrain_blend::BlendCensus` (aid only; `world.rs::blend_census`) before any
selection: reference and base form IDs, model path, the shape's index in the model (for a
rigged STAT, its index among the shapes `spawn_nodes` drew), and a reason mask. Reference
reasons: `world` (not an exterior cell of the drawn world), `rig` (rig or skinning),
`particles`, `posed` (jointed, loose or stored pose), `damage-stages`, `dest`, `distant`
(imposter). Shape reasons: `emittance` (external emittance), `morph` (registered morph
mesh). Only shapes with an empty mask get a `BlendShape`, as before (the selection is the
same: `eligible_reference` and the old `blend_candidate` test are now derived from the
mask), so tracking never makes a shape convertible. The conversion adds, each frame:
`missing-source` (the material is not loaded), `temporary` (an active
`nvf_play::fade::Fading`, `effect_shaders::Faded` or `MaterialOverride`), and one reason per
material predicate: `alpha-mode` (Add, Multiply, Premultiplied, AlphaToCoverage), `unlit`,
`refraction`, `first-person`, `depth-write`, `specular-transmission`,
`diffuse-transmission`, `falloff`, `holes-value`, `holes-texture`, `facegen`, `pipboy`,
`add-linear`, `soft`, `depth-bias-nonfinite` (`mesh_terrain_blend::reason`; the bit order
is the first-reason precedence).

Lines (grep `mesh terrain blend: census-shape` and `mesh terrain blend: ready`):

```text
mesh terrain blend: census-shape ref=0x%08X base=0x%08X model=<path> shape=<n> status=eligible|excluded first-reason=<name|none> reasons=<a+b|none>
mesh terrain blend: ready eligible-shapes=N resident-cells=N generation=N variant=V shape=S resolution=R shading=H <weight fields> stat-shapes=N excluded-shapes=N pending-material-shapes=N excluded-<reason>=N ... admitted-blend=N admitted-bias=N admitted-decal=N admitted-mask=N admitted-alpha-cut=N
```

A `census-shape` line is printed once per distinct base form, shape index and mask (the
reference is the first seen with them), not while the material is still loading, at most
2048 per run (then one `census-shape lines stopped` notice). The ready line's counts are
the currently resident tracked shapes, not unique materials or cumulative spawns; reason
counts overlap and all 26 are printed, zeros included; `pending-material-shapes` equals
`excluded-missing-source`; `admitted-*` count the enabled shapes whose source is Blend,
has a nonzero depth bias, the decal flag, is Mask or has an alpha cut. The ready line is
printed at each publication (also with zero eligible shapes) and refreshed when the
census changes, at most once per 60 frames. A road with no `census-shape` line is not a
tracked STAT shape (another record type, or not drawn through `spawn_model`): no material
rejection may be inferred.

[design] The colour-coded `excluded` view was not built: it needs a new field in the
ordinary `NifSurfaceKey` (every NIF pipeline), owned diagnostic clones of rejected
materials, and a restore-before-producers system interleaved with fades, swaps, overrides
and NIF edits. The writer judged that not contained for this round; `excluded` is not an
accepted view, and the `census-shape` lines classify a shape instead (grep its model path).

### Admissions

[design] Now admitted when otherwise eligible: ordinary `AlphaMode::Blend` (the blend
material keeps the source's alpha mode, so its transparent phase, blend state, sorting
and depth-write state stay the shared specialization's; the shared `depth_equal` skips
blended targets), a finite nonzero `depth_bias` (copied exactly; the shared
specialization applies the raster bias), the lit mesh `decal` flag (its bias stays the
shared specialization's) and `alpha_cut` (the discard runs before terrain shading; copied
unchanged). Mask was already admitted. Holes and FaceGen stay mandatory exclusions (their
slots are repurposed); Add, Multiply, Premultiplied and AlphaToCoverage stay out.

[design] Because a fade copy is an ordinary Blend material, ownership is now tested
explicitly (`temporary` above) instead of through the alpha mode; restoration by the
owners is unchanged. The conversion is ordered after
`nvf_assets::nif_material::BlendedCutoutSystems` (new set holding `apply_blended_cutouts`),
and each enabled clone records a stamp of its source (alpha mode, alpha cut, colour alpha,
depth bias, decal, roughness, blended cut-out): a same-handle edit (the cut-outs flipping
Blend and Mask with FSR, NIF block edits) rebuilds the clone in the same frame without a
new ordinary component. When a clone is (re)assigned, the see-through shadow markers are
refreshed by the shared rule (`see_through`), since the shared system only sees ordinary
component changes.

[hypothesis] Risks: a 48-64 unit band may flatten rock lighting (compare 32 first, keep
`albedo`); very shallow shoulders may want a larger flat width, which also reaches nearly
parallel surfaces; interpolated mesh normals and faceted LAND normals vary the width across
triangles; admitted Blend and decal shapes (glass, markings) near the ground can take the
landscape's colour; wider bands cannot repair a wrong lookup or a shape that is not
eligible, which the census separates.

### Tests, type-checked only

- `nvf-assets` `nif_surface_blend::tests`: round 5's shapes unchanged and blind to the new
  widths (`hybrid-v1` = round 5's hybrid); v2 flat, shallow and steep plateaus, monotonic
  ramps; continuity in height and angle on a 0.1 grid; saturation below ground; zero
  weight on parallel tops at 0.5-15 units and slightly tilted tops at 10-15; width
  validation; the new shader definitions; 176/128-byte uniforms.
- `nvforever` `mesh_terrain_blend::tests`: widths parsing (defaults, the comparison set,
  every rejection, old forms kept); start/ready lines with their prefixes and all census
  fields; `census-shape` format, dedupe, pending skip and cap; every reference, shape and
  material reason alone and overlapping; admission combinations (Opaque/Mask/Blend with
  bias, decal, alpha cut, hard edges, blended cut-out) preserving every descriptor; ECS:
  census through publication with zero eligible, late arrival, a pending material
  loading, a same-handle source edit, removal; authored Blend admitted while an override
  owns it and after; authored Blend through a script fade (the copy is material-eligible,
  Fading keeps it out); same-handle blended cut-out flips in both directions, no clone
  nesting; same-handle see-through edits refresh the shadow markers. Round 5's swap, fade,
  shadow and NIF-edit tests are kept. `effect_shaders::Faded` has private fields, so its
  exclusion is covered by type only.
- `nvforever` `mesh_blend_shader_tests`: 176-byte enabled struct; the positive-weight
  branch holds every shading change; the WESL weight transcribes the CPU statement.

[documented: round-6 command outputs] From `engine` with
`CARGO_TARGET_DIR=E:/Games/NVForever/engine/target-2661-opus`: `cargo check -p nvf-assets
--tests`, `cargo check -p nvforever --tests` and `cargo check -p nvforever` exit 0 without
warnings. `python tools/smoke verify`: exit 0, `scenario file: 1519 scenarios, fine`. No
cargo build/test/clippy, game run, smoke run or game-data access.

### Smoke

[design] `mesh-terrain-blend-goodsprings` (value unchanged, `24:surface`) also requires the
start line's `flat=1.5 steep=144 weight-version=2`, a ready line with the census after
round 5's fields, the admitted counts, an eligible `census-shape` line and at most 2048
`census-shape` lines. `mesh-terrain-blend-options` also requires `weight-version=1`. New
`mesh-terrain-blend-widths` runs `8:weight:hybrid:1024:full:0.5,32` and requires the
resolved widths, a ready line with them and the census, `census-shape` lines, no
rejection and no shader errors. Existing expectations, names and values are unchanged.

### Comparison set for the main session

[design] Identical camera, noon, 1280x720, publication and fades settled, at view A (the
round-3/4/5 road shoulder) and view B (the rock foot): aid off once per view, then

| `NVF_PLAY_TEST_MESH_BLEND` | Purpose |
| --- | --- |
| `8:surface:height:1024:full` | the successful shoulder reference |
| `8:surface:hybrid:1024:full:0.5,32` | narrow rock band |
| `8:surface:hybrid:1024:full:0.5,48` | proposed default (= `8:surface`) |
| `8:surface:hybrid:1024:full:0.5,64` | wider rock band |
| `8:weight:hybrid:1024:full:0.5,48` | shoulder, top and rock weight distribution |

The plan's `8:excluded:...` frame is replaced by the B road's `census-shape` line (grep
its model path in the log of any run) and the ready line's counts; `8:selection:hybrid:
1024:full:0.5,48` shows which shapes are converted. Judge A for shoulder continuity and an
unchanged asphalt top; B for a visible rock-foot transition and the road's classification.
Keep the start, ready and census lines beside each frame. If the road is excluded, its
reason comes before further width tuning; `8:surface:hybrid-v1:1024:full` reproduces
round 5's frames.

[design] Main session: the `nvforever` filters `mesh_terrain_blend`,
`mesh_blend_shader_tests`, `terrain_surface`, `terrain::tests`, `texture_swap`,
`nif_mirror`, `fade`, `draw_distance`, `door_fade`, `effect_shaders`, `shadow_thin`; the
`nvf-assets` filters `nif_surface_blend::tests`, `nif_material::tests`; the `nvf-play`
filter `fade`; engine clippy; smoke `mesh-terrain-blend-goodsprings`,
`mesh-terrain-blend-options`, `mesh-terrain-blend-widths` and round 2's Off checks.
