# Terrain shadows beyond the sun's cascades (#2671)

An enhanced-look feature, off by default, not a parity item: the landscape shades itself
where the sun's shadow cascades no longer reach (`--shadow-distance`, 8,000 units by
default). A page of its own rather than a section of `generated-lod.md`: that page is about
the distant meshes' bake, while this feature has its own height source, a GPU producer and
a lighting term, and draws on the generated LOD only as one possible height source.

Stages: T0 the height source (`engine/nvforever/src/sun_heightfield.rs`), T1 the
shadow-height producer (`engine/nvforever/src/terrain_shadows.rs`, `terrain_shadows.wesl`),
T2 the receivers (`engine/nvforever/src/sun_visibility.wesl`, through the sun visibility of
[sun-visibility.md](sun-visibility.md)) and the settings row Terrain shadows (experimental)
(`terrain-shadows`, Graphics › Shadows, off by default and in every preset).

## T0: the height source

A world-aligned map of 512 × 512 texels of 256 units (131,072 units, 32 × 32 cells) around
the camera's cell, each texel the highest stored LAND vertex on its square, with a validity
mask (no LAND, not loaded, outside the worldspace: invalid, height −1e30). Published as an
`Rg32Float` image (R height, G validity), row 0 the southernmost, with its origin, texel
size, side, valid height range and a version. Details and bounds in the module notes of
`sun_heightfield.rs`.

## T1: the shadow-height map

**What it computes.** For each texel, the shadow height
`S(x, z) = max over t > 0 of [H((x, z) + t·L) − t·tan(e)]`, L the horizontal unit direction
towards the light, e its elevation. A receiver at height y over that texel is in the
landscape's shadow when y < S. One value per texel serves every receiver height (ground,
roofs, actors, leaves, water), which a horizon angle stored at ground level would not
[engineering choice, #2671 plan].

**Published basis.** Storing per-texel light occlusion of a height field along a light
direction is the idea of horizon mapping [documented: N. Max, "Horizon mapping: shadows for
bump-mapped surfaces", The Visual Computer 4(2), 1988]. A max-height mipmap lets a march
over a height field skip space without stepping over a ridge [documented: A. Tevs, I. Ihrke,
H.-P. Seidel, "Maximum mipmaps for fast, accurate, and scalable dynamic height field
rendering", I3D 2008]; T1 does not build one yet (below). The shadow-height form, the step
schedule and the cadence rule are engineering choices for this engine, not taken from
those papers.

**The pass** (`terrain_shadows.rs` module notes): a compute pass, one invocation per texel
(8 × 8 workgroups), reading T0's image with `textureLoad` and writing an `Rg32Float` storage
texture (`Rgba32Float` since T2): R = S, G = the horizontal distance to the texel that gave
S (T2's penumbra), −1e30 / 0 where nothing casts, B = the ground's relief around the texel
(T2's bias). `Rgba32Float` write-only storage is a guaranteed format in wgpu 30 without
optional features [documented: wgpu-types 30.0.2
`TextureFormat::guaranteed_format_features`]. It runs in the render graph's `Begin` set,
only on a frame with a new job; the result is published when the render world has recorded
the whole dispatch, into one of two targets (the other stays published meanwhile).

**The march** [engineering choice]. From the receiver texel's centre, at most 96 samples at
`t_i = 0.5·i + b·i²` texels, b set so the last sample lands on the ray's reach: the nearer
of the map's edge and `(max − min) / tan(e)`, past which nothing on the map can rise above
its lowest height. The samples are held 1e-3 texel short of the reach, so the last one lies
in the last texel the ray crosses inside the map, never on the edge itself (until
2026-11-17 it fell on or past the edge and ended the march, so a caster in the last step
before the edge cast nothing). High sun therefore costs a few samples, and low sun covers the whole map
with steps that grow as the square root of the distance. Every sampled texel is scored with
the ray's exact entry distance into it, as the brute-force CPU reference
(`sun_heightfield::shadow_height_reference`) scores it, and the march stops once a ray
falling from the map's highest height cannot beat the best so far. The receiver's own texel
is left out (its maximum is the receiver's own ground), as in the reference.

**Bounds.**
- The march visits a subset of the texels the reference visits and scores them alike: it
  is never darker than the reference (tested: `march_matches_reference_on_ridge`).
- It is lighter only where it steps over a texel. The longest step is 0.5 texel while the
  reach is under 48 texels and 14.5 texels (3,700 units) at the full diagonal of 724 texels
  (`max_step`, test `steps_grow_with_reach`). A caster at least as long along the ray as the
  local step is always sampled, and for a flat-topped caster S is then within
  `tan(e) · step` of the reference (tested on synthetic ridges); a narrower caster can be
  missed. A caster the map's edge cuts shorter than the step is still sampled, by the last
  sample (only a sliver under 1e-3 texel is missed; tests
  `march_reaches_casters_near_the_map_edge`, `march_matches_reference_at_low_sun_from_every_side`).
  A receiver standing on a caster can see the caster's rest as narrower than the step, when
  its ray leaves it sideways right after the receiver's own texel. Replacing the texel read with a max-pyramid read at the step's level would make
  the march conservative instead; that is the planned refinement if pictures show leaks.
- The heightfield keeps each texel's highest point, so on a slope facing the light S can
  stand up to one texel's relief above the receiver's own ground (test
  `slope_overshoot_is_within_a_texel_of_relief`): T2 needs a slope-scaled bias of about
  `texel · |slope|` (as built: "The bias" under T2).
- Rays end at the map's edge: a ridge beyond it casts nothing. With T0's recentring the
  camera stays at least 11 cells (45,056 units) from the edge.

**Cadence** [engineering choice, #2671 plan]. A new map only when T0's version changes or
when the predicted shadow-edge displacement since the last map exceeds half a texel (128
units), never sooner than 0.1 s after the last one. For a blocker of height h (the map's
`max − min`, a bound for every blocker on it) the shadow's length `h·cot(e)`, at most the
map's diagonal, changes by its difference (to first order `h·csc²(e)·Δe`), and its tip
swings by `length · Δazimuth`. The light is the sun entity's own transform, which turns every
frame. For h = 2,000 units the elevation alone allows 0.028° at 5°, 0.25° at 15° and 1.8°
at 45° before a new map (test `low_sun_updates_more_often`); the sun climbs about 0.21° a
game minute near the horizon (14 hours from 06:00 to 20:00 over 180°, `fnv_world::sky::sun_direction`),
so about every 8 game seconds at 5°, every 70 at 15° and every 9 game minutes at 45°, the
first held to the 0.1 s floor [derived from the formula, not measured]. Below the horizon
fade where the sun's own shadow maps stop (sine of the elevation under 0.1 · sin 0.1,
`sky.rs` `HORIZON_FADE`, `graphics.rs` `SHADOW_FADED`) nothing is computed and "no terrain
shadow" is published; at night the light is the moon and is handled alike.

**Off path.** Everything sits behind `TerrainShadowsWanted` (false by default; the settings
row or the debug override sets it, T2), which turns T0 on with it. While false no job is made, no target allocated, no pipeline compiled, and
the render system's run condition fails.

**Not measured yet.** The pass's GPU time (it carries a `terrain_shadows` time span), how
often low sun asks for a new map in play, whether the march's misses or the heightfield's
max-per-texel are visible, and the producer on the Steam Deck class of GPU. The cost model
in the plan (about 25 million height reads for a full map at 96 samples) is an estimate
[hypothesis].

**Debug aids.** `NVF_PLAY_DEBUG_TERRAIN_SHADOWS=1` turns the producer (and T0) on;
`NVF_PLAY_DEBUG_TERRAIN_SHADOW_DUMP=<path.png>` writes the first map published 3 s into the
run, its heightfield and a shaded-ground mask as greyscale PNGs, north up.

## T2: the receivers

**Where.** sun_visibility.wesl `nvf_terrain_visibility` gives T for a receiver at world
position P lit from L; the sun visibility's composition (docs/design/sun-visibility.md §2)
uses it: `V = K · Vsolid`, `Vsolid = min(C, T)` nearer than the last cascade's end band,
T beyond it. It runs in the directional light loop of the patched `apply_pbr_lighting`, on
the outdoor sun (the moon at night) only, for receivers and non-receivers alike (terrain,
objects, actors, grass, leaves, tree cards, water). The CPU mirrors in sun_visibility.rs
(`terrain_visibility`, `terrain_lit`, `solid_visibility`) follow the shader line for line;
the constants are in both, a test compares them [design: the files].

**The map as read** (`terrain_shadows.rs` module notes): `Rgba32Float`, R = S, G = the
caster's horizontal distance, B = the ground's relief around the texel (below), row 0 the
southernmost; `uv = P.xz · uv_scale + uv_offset`, texel (i, j) spans uv `[i, i+1]/side`.
Where nothing casts, and on texels with no terrain, R = −1e30 and B = 0.

**The filter.** The texel-space position `x = uv · side` is taken half a texel back
(`c = x − 0.5`); the four texels around it (`floor(c)` and `+1`, indices clamped to the map,
since the shared sampler repeats and the format is not filterable) are read with
`textureLoad`, each is *compared* with the receiver, and the four comparisons are weighted
bilinearly by `fract(c)`. S itself is never interpolated: it jumps by hundreds of units at a
ridge's shadow end, and an interpolated S would shade the ground in front of a ridge's
shadow [engineering choice; the percentage-closer idea of filtering comparisons, not
depths: W. Reeves, D. Salesin, R. Cook, "Rendering antialiased shadows with depth maps",
SIGGRAPH 1987].

**The soft edge** [engineering choice]. Each comparison is
`lit = smoothstep(0, 1, saturate((y′ − S) / band + 1))`: fully lit at and above S, fully
shaded `band` under it, where

`band = max(G · 0.01, 0.25 · texel) + 0.5 · texel · min(tan e, 1)`

- `G · 0.01`: the penumbra grows with the caster's distance; 0.01 rad stands for the sun's
  apparent diameter (about 0.53°, 0.0093 rad, rounded). At G = 20,000 units: 200 units of
  height.
- `0.25 · texel` (64 units): a floor, so a near caster's edge is not razor sharp at the
  256-unit grid.
- `0.5 · texel · tan e`: half of what S falls along the light over one texel, so the four
  comparisons change smoothly across a texel and the grid does not show as stair steps;
  `tan e` (from L) is held to at most 1, as a high sun shades little and would widen the band
  past a texel's fall.

The band lies *under* S, not around it: a receiver at S is lit. That makes every shadow
shorter by about half a band (`band / (2 · tan e)` horizontally: about 200 units at 07:00
behind a near ridge) and keeps the comparison from shading a lit slope [engineering choice:
the brief prefers too little shadow to speckle].

**The bias** [engineering choice]. Before comparing, the receiver is lifted:

`y′ = P.y + 16 + 2 · relief`, `relief` the bilinear mix of the four texels' B.

Why: T0 keeps each texel's *highest* vertex, so on a slope S over a texel stands up to about
a texel's relief above the ground there (test `slope_overshoot_is_within_a_texel_of_relief`);
unbiased, lit slopes speckle and stripe (the main session's dump at Goodsprings 07:00). For a
plane rising g per unit towards the light (lit while g < tan e), the sunward neighbour's
maximum gives `S − y ≤ (1.5 − f)·g·τ − 0.5·τ·tan e` at the receiver's own texel (f the
receiver's fraction towards the next centre, τ the texel) and `(2.5 − f)·g·τ − 0.5·τ·tan e
< (2 − f)·g·τ` at the next one; a plane facing the light at slope s gives at most
`(f − 0.5)·s·τ`; a slope across the light adds half a texel's cross relief. With relief = g·τ
on an axis-aligned slope, `2 · relief` covers every case; 1.5 did not (a prototype of the march
and the receiver left T at 0.991 on a lit slope; test `lit_slope_has_no_acne`: planes up to
45° from eight directions under suns at 5°, 13° and 30°, T > 0.9999 everywhere, while the
unbiased comparison shades the ground at over 10,000 of the samples). The 16 units cover
f32 noise and the rendered triangles lying under their highest vertex.

What **relief** is (`terrain_shadows.wesl` `relief`, mirrored by `texel_relief`): per axis
(east-west, north-south) the *smaller* of the two neighbours' height differences (one side
alone at the map's edge or beside an invalid texel, 0 when both are missing), the two axes
summed; 0 for an invalid texel. A uniform slope gives `τ·(|g_x| + |g_z|)`; the flat foot (or
flat top) of a cliff gives 0, so the bias does not lift a cliff's own shadow off the ground
right at its foot (test `valley_behind_ridge_is_shaded`: shaded from 0.2 texel past the
ridge). Why not the alternatives the plan named [engineering choice]: the hook passes no
normal (`nvf_sun_visibility(P, L, C, view_z)` in three callers), and a receiver's own normal
is not the ground's (a wall facing the sun would get an endless bias and never take a ridge's
shadow); S's own gradient along the light is tan e inside every real shadow, so a bias from it
would shorten every shadow by its factor times a texel. The relief costs the producer four
more loads a texel per map and widens the output from `Rg32Float` to `Rgba32Float` (4 MB a
target at 512 × 512, two targets).

**Edges.** T fades to 1 over the map's last 4 texels (1,024 units; smoothstep of the
distance to the nearest edge) and is 1 outside the map, before any texture read
[engineering choice: marches end at the edge, so near it nothing is known]. Texels with no
terrain hold S = −1e30 and shade nothing (test `invalid_texels_shade_nothing`); the bilinear
filter fades a shadow into such a hole over one texel. Test `map_edge_fades_out`.

**Only terrain-scale shadows; the band.** T is the landscape's shadow only (256-unit
texels): near the camera the cascades shade objects and the terrain alike, and `min(C, T)`
counts a ridge already in both once. The band over which Vsolid goes from `min(C, T)` to T is
the last part of the sun's last cascade, as long as Bevy's overlap between cascades
(`overlap_proportion` 0.2 of the far bound: 6,400 to 8,000 units at the default shadow
distance; `sun_visibility.rs` `cascade_fade_band`, fed from the `Sun` entity's
`CascadeShadowConfig` while its shadow maps are on) [engineering choice]. It was inert until
T2 (no terrain variant drew); with a map published it is active (test
`terrain_band_blend_is_active`: a cascade shadow on lit terrain fades out across it). With
Sun shadows off, C = 1 and Vsolid = T at every distance; the band is then 0 and changes
nothing.

**Activation** (`terrain_shadows.rs` `feed_sun_visibility`, `follow_graphics`): the settings
row Terrain shadows (or `NVF_PLAY_DEBUG_TERRAIN_SHADOWS=1`, which wins) sets
`TerrainShadowsWanted`, which turns T0 and T1 on and `SunVisibility::terrain_enabled`, which
alone decides the views' terrain variant, markers and pipeline key bits. The publication only
fills `SunVisibility::terrain` (the uniform and the texture). Switched on with nothing
published (indoors, no heightfield yet, the light under the horizon fade, the first map still
computing) the uniform's terrain bit is clear: T = 1 and Vsolid = C before any read, and
binding 41 holds a one-texel stand-in that casts nothing (S = −1e30; Bevy's white fallback
would read S = 1 and shade every receiver below y = 1). So doors, the sun setting and new
maps never rebuild pipelines; only the switch does (test `terrain_bindings_follow_the_switch`,
smoke `terrain-shadows`).

**Freshness.** The image is bound by handle: T1's two targets alternate, and
`prepare_sun_visibility` compares the bound texture view with the uploaded one every frame,
so each publication is read from the frame it is extracted on. Between two publications the
map stands still while the sun moves: the step is accepted, not blended [engineering choice:
the simple one]. T1's cadence bounds it: a new map once the predicted shadow-edge movement
for the map's whole relief (a bound for every blocker on it) exceeds half a texel (128
units), at most every 0.1 s; so a shadow's edge jumps by about 128 units at most when a new
map lands, plus what the light moves during the frame or two a job takes to be published.
The 0.1 s floor holds maps back only when the edge would move 128 units in under 0.1 s: at
5° and 2,000 units of relief that is 0.028° of sun, about 0.27 s of real time at the game's
default TimeScale of 30 (sky.rs, citing the GECK wiki), so it binds only with a faster clock [derived from T1's cadence
formula, not measured].

**Debug view.** `NVF_PLAY_DEBUG_SUN_VISIBILITY=terrain` draws T alone (white lit, black
shaded) in place of the lit colour; `combined` draws V = K · Vsolid. Without the row (or the
override) T is 1 everywhere: the view turns the plumbing on but not the terrain variant.

**Cost** [hypothesis, unmeasured]: four 16-byte texel loads, four smoothsteps and a few
multiplies per fragment of the sun's light loop, twice where the transmitted lobe is drawn;
the producer's four extra loads per texel per map.

**Not measured or shown yet** (only a run can): that the patched lighting compiles with the
unfilterable binding in every pipeline of a marked view and that the layouts match on the
reflection and scope views; that ridges shade valleys where the dumps say and lit slopes stay
clean in pictures; that the 256-unit grid does not show; how the band looks at the cascades'
end; the per-fragment cost.

**Open for tuning** (sun_visibility.wesl and sun_visibility.rs `TERRAIN_*`, changed together):
- the penumbra per unit of caster distance (0.01) and its floor (0.25 texel);
- the grid term (0.5 texel times tan e, capped at tan e = 1);
- the bias (2 × relief + 16 units) against how short it makes shadows on slopes;
- the band's place under S (lit at S) against a band centred on S (truer edges, more risk
  of stripes on slopes);
- the edge fade (4 texels);
- the fade band (the last cascade's overlap share, 20 %);
- whether the moon should cast terrain shadows at night at the same strength.
