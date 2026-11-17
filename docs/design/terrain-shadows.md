# Terrain shadows beyond the sun's cascades (#2671)

An enhanced-look feature, off by default, not a parity item: the landscape shades itself
where the sun's shadow cascades no longer reach (`--shadow-distance`, 8,000 units by
default). A page of its own rather than a section of `generated-lod.md`: that page is about
the distant meshes' bake, while this feature has its own height source, a GPU producer and
a lighting term, and draws on the generated LOD only as one possible height source.

Stages: T0 the height source (`engine/nvforever/src/sun_heightfield.rs`), T1 the
shadow-height producer (`engine/nvforever/src/terrain_shadows.rs`, `terrain_shadows.wesl`),
T2 the receivers and the settings row (not built yet).

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
texture: R = S, G = the horizontal distance to the texel that gave S (for a later penumbra
term), −1e30 / 0 where nothing casts. `Rg32Float` write-only storage is a guaranteed format
in wgpu 30 without optional features [documented: wgpu-types 30.0.2
`TextureFormat::guaranteed_format_features`]. It runs in the render graph's `Begin` set,
only on a frame with a new job; the result is published when the render world has recorded
the whole dispatch, into one of two targets (the other stays published meanwhile).

**The march** [engineering choice]. From the receiver texel's centre, at most 96 samples at
`t_i = 0.5·i + b·i²` texels, b set so the last sample lands on the ray's reach: the nearer
of the map's edge and `(max − min) / tan(e)`, past which nothing on the map can rise above
its lowest height. High sun therefore costs a few samples, and low sun covers the whole map
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
  missed. Replacing the texel read with a max-pyramid read at the step's level would make
  the march conservative instead; that is the planned refinement if pictures show leaks.
- The heightfield keeps each texel's highest point, so on a slope facing the light S can
  stand up to one texel's relief above the receiver's own ground (test
  `slope_overshoot_is_within_a_texel_of_relief`): T2 needs a slope-scaled bias of about
  `texel · |slope|`.
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

**Off path.** Everything sits behind `TerrainShadowsWanted` (false by default), which turns
T0 on with it. While false no job is made, no target allocated, no pipeline compiled, and
the render system's run condition fails.

**Not measured yet.** The pass's GPU time (it carries a `terrain_shadows` time span), how
often low sun asks for a new map in play, whether the march's misses or the heightfield's
max-per-texel are visible, and the producer on the Steam Deck class of GPU. The cost model
in the plan (about 25 million height reads for a full map at 96 samples) is an estimate
[hypothesis].

**Debug aids.** `NVF_PLAY_DEBUG_TERRAIN_SHADOWS=1` turns the producer (and T0) on;
`NVF_PLAY_DEBUG_TERRAIN_SHADOW_DUMP=<path.png>` writes the first map published 3 s into the
run, its heightfield and a shaded-ground mask as greyscale PNGs, north up.
