# Mesh terrain blend, rounds 1-4 (#2661)

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
