# Enhanced graphics: engine-level improvements (workstream L)

Status: first set built 2026-10-12 (branch `claude/graphics-engine`, PR #236); follow-ups
(temporal anti-aliasing, sun shafts, screen-space water reflections; Bevy's volumetric fog tried
and dropped) the same day (branch `claude/graphics-2`). Owner's request (playtest
list 2, docs/handoff/archive/playtest-2026-10-12.md): "Add any modern engine level graphics rendering
functionality to improve the game visual without modifying the base assets or implementing new
ones."

Everything here is an **engineering choice**, not a fact about the original game: none of it
is tagged `[documented]`/`[observed]` about Fallout: New Vegas, and none of it may be used to
explain a difference against the game's own frames. The parity checks compare the `vanilla`
look only.

## 1. What the renderer does now (Bevy 0.19.1, `engine/nvforever`)

| Part | State before this work | Where |
|---|---|---|
| Main view | Not HDR: forward PBR with tonemapping (Tony McMapface) in the shaders into an 8-bit sRGB target; MSAA 4x (`--msaa`, the owner's game runs 8); depth prepass (water and the image-space blur read it) | `main.rs` `setup` |
| Alpha-tested edges | Smoothed with MSAA (soft edges standing in for transparency multisampling) | `nvf_assets::nif_material`, grass, trees |
| Sun | One directional light; shadows on by default in the game since 2026-10-26 (owner, once the first-person flicker was fixed: player-body.md; off 2026-10-13 to 10-26 for frame cost and that flicker; `--no-sun-shadows`, off without `--play`; `--sun-shadows` or the settings page): 3 shadow cascades out to `--shadow-distance` (8,000 since #254; §5.3), maps 2048² (Bevy's default), Gaussian PCF (Bevy's default); a second "shade" light for the first-person arms casts within 256 units of the camera (`ARMS_SHADE_REACH`) | `sky.rs`, `player_body.rs` |
| Local lights | Point/spot lights from LIGH records, flashlight with shadows | `lights.rs`, `flashlight.rs` |
| Sky | The game's sky meshes, colours mapped through the inverse of the tonemapper so the grade sees the weather's colours | `sky.rs`, `sky.wesl` |
| Fog | Bevy distance fog, linear, from the weather's near/far | `sky.rs` |
| Water | Planar reflection camera, refraction from the view transmission copy, depth-based shore fade | `water.rs`, `reflection.rs` |
| Image space | The game's grade (saturation, tint, brightness, contrast) and the modifiers' blur, double vision, radial blur, fade, as passes after tonemapping | `imagespace.rs` |
| Texture filtering | Trilinear, 8x anisotropic for the game's DDS textures; trilinear (no anisotropy) for the default sampler | `nvf_assets::texture` |
| Bloom, AO, eye adaptation | None (image-modifiers.md §5: the HDR and bloom parameters are not drawn) | |

Constraints that decide what fits:

- The custom prepass fragments (NIF, grass, tree, water) write **depth only**; Bevy's SSAO, SSR,
  TAA and motion blur need normal or motion-vector prepasses from every material.
- The view is **multisampled**, and the alpha-tested foliage depends on it. Bevy's SSAO and TAA
  require MSAA off.
- The owner's PC has a **GeForce GTX 1050 (4 GB)**, shared with other sessions' game runs.
- `vanilla` must stay the picture the parity lab and the smoke checks were made with.

## 2. Candidates

| Candidate | Value | Cost / risk | Decision |
|---|---|---|---|
| Ambient occlusion (own pass from the depth prepass) | High: grounds objects, adds depth to interiors | ~1-2 ms; darkens direct light too (post-process) | **Built** (§3.1) |
| Bevy SSAO (GTAO-like) | High | Needs MSAA off + normal prepass in every custom prepass shader | Rejected |
| Longer sun shadows (reach ×1.5; 4096² maps until #1367) | Medium: shadows further out | About 0.2-0.3 ms at Novac for the longer reach; the 4096² maps cost about 2 ms more and went | **Built** (§3.2) |
| Contact shadows (Bevy, screen-space ray march) | Low-medium | Tried: speckled self-shadowing on near surfaces and per-frame noise without TAA | Rejected (§4) |
| Bloom (own LDR pass, thresholded) | Medium: lamps, windows, the sun glow as in the game's own frames (world.md §6 notes the halos the renderer lacks) | ~0.5 ms at quarter size | **Built** (§3.3) |
| Bevy bloom | Medium | Needs an HDR main view: Rgba16Float targets (twice the traffic with MSAA 4x on the 1050) and every pass reading the picture runs on another format | **Built** 2026-10-23 as the light glow, on in vanilla (§3.11; the owner's PC now has an RTX 4070) |
| Sharpening (own contrast-adaptive pass) | Medium: the 2010 textures look crisper | ~0.2 ms; can add rims if overdone | **Built** (§3.4) |
| Bevy CAS | Medium | Tried: garbled the non-HDR view (yellow/white noise) | Rejected |
| Anisotropic filtering 16x | Low-medium: ground and roads at a slant | Nearly free on the GPU | **Built** (§3.5) |
| Sun in-scattering in the fog (Bevy `DistanceFog` directional light) | Low: NVWastelandClear's fog reaches 200,000 units, so nothing showed | A shadow-map fetch per fragment | Tried, rejected |
| Bevy TAA | Medium | Needs motion vectors from every material's prepass (the custom NIF, grass, tree and water prepasses write depth only; wind-animated grass and trees would need last frame's wind) and MSAA off | Not used: own pass instead |
| TAA from depth and camera alone (own pass) | Medium: smooth edges and plant cut-outs without multisampling, cheaper than MSAA 4x | Softer textures while the view moves; what moves by itself is not followed | **Built**, as the other choice to MSAA, not in `enhanced` (§3.6) |
| Bevy FXAA, Bevy SMAA | Low-medium: smooth edges without multisampling, cheaper than MSAA; on top of MSAA they add little | Blur fine texture detail (FXAA more); plants' cut-outs are not smoothed (no multisampling) | **Offered** in the anti-aliasing choice instead of MSAA, not in `enhanced` (§3.6, 2026-10-16) |
| Shadow filtering method (Bevy `ShadowFilteringMethod`) | Low: harder or softer sun-shadow edges | Temporal is grainy without TAA | **Offered** as a choice, Bevy's default (Gaussian) kept (§3.9) |
| Sun shafts (own screen-space pass), tied to the weather's fog | Medium: dusk and dawn, the sun behind roofs and poles | ~0.03 ms here | **Built** (§3.7) |
| Bevy volumetric fog | Would replace the game's fog look | Tried: darkens the whole picture on the non-HDR view, box edges | Rejected (§4) |
| SSR on water | Low: the planar reflection is better where it reaches | Own march in the water shader from the depth prepass | **Built** for where the planar reflection does not reach (§3.8) |
| Bevy atmosphere | Would replace the game's sky look | | Rejected |
| Auto exposure, depth of field, motion blur | Change the look; the game's own HDR/eye-adapt values (IMGS DNAM) are not decoded | | Not now (§6) |
| PCSS soft shadows | Medium | Needs the `experimental_pbr_pcss` feature, noisy without TAA | Not now |
| Placed meshes blended into the landscape where they meet it (#2661) | Medium: road shoulders, rock feet and cliff bases lose their hard seam with the ground | Immutable front/back colour atlases (144 MiB together at 1024 before intermediates and retiring handles); copies for unchanged cells and rasterization for entering/dirty cells; a lookup in eligible fragments; round 10a: about 0.66 MiB more height atlas per publication (detail rows), 1.6 KB more index table per enabled clone, and near contact fragments read the landscape's own layer textures (up to 13 diffuse and 13 normal reads) | **Built**, off by default and not in `enhanced` (§3.12) |

## 3. What was built

`--graphics vanilla|enhanced` picks the look; each improvement is also a switch on the pause
menu's settings page (category "Graphics") and can be compared alone with
`--graphics-only <keys>`. The settings page's **Enhanced effects** row (Graphics > Picture, #1966) sets
them all as `enhanced` (On) or `vanilla` (Off) does and reads Custom when they are mixed;
the Quality row's presets never touch them (docs/spec-notes/quality-presets.md). The log says what is on: `graphics: ambient occlusion, longer
shadows, bloom, sharpening, 16x anisotropic filtering, sun shafts, screen-space water
reflections` (or `graphics: vanilla (no engine-level improvements)`), again whenever a switch
changes. Anti-aliasing (`anti-aliasing`: multisampling off/2x/4x/8x, FXAA, SMAA or TAA) and
shadow filtering (`shadow-filter`) are choices of their own, not part of `enhanced` (§3.6,
§3.9); the field of view, the display sync and a frame-rate limit are on the Display page (§3.10).
"Blend meshes into ground" (`mesh-terrain-blend`, §3.12) is a switch off in both looks and
left out of `enhanced` until the maintainer decides.
Code: `graphics.rs` (preset, switches, applying them), `ssao.rs`/`ssao.wesl`,
`post.rs`/`post.wesl`, `taa.rs`/`taa.wesl`, `shafts.rs`/`shafts.wesl`, `water.wesl` (`ssr`),
`sky.rs` (`shadow_reach`), `nvf_assets::texture::set_anisotropy`, `frame_pace.rs`, the rows in
`settings.rs`.

[design: `engine/nvf-play/src/graphics_resume.rs`, `settings.rs`, `pause.rs`;
`engine/nvforever/src/settings.rs`, `pipeline_gate.rs`] **Resuming after graphics edits
(pipeline portion of #1820).** Effective edits to the registered renderer rows latch for
the pause session, including Defaults. Continue, Esc and the timed resume aid use one
request. An unchanged session resumes immediately. A changed session keeps
`PlayState::Menu`, `Paused(true)` and the pause picture while rendering and pipeline
compilation continue. The wait counts all `PipelineGate::busy()` pipelines, including
sun shadows even with the travel-only shadow test aid enabled.

[design: the same files; `engine/nvf-play/src/loading.rs`] After the resume request and
after any further edit, fresh completed render frames must report no pending pipelines
and no new requests for at least 0.25 real seconds and three render frames. A stale zero
count cannot release the menu; a late request restarts settling even if it has already
finished. The wait ends at the loading screen's `PIPELINE_CAP_SECS` (15 real seconds)
if readiness never arrives. Repeated resume requests keep the first deadline. Loading a
save, quitting or closing the menu clears the session. Choosing anything else on the
column while the wait runs (Save, Load, Settings, Mods, Quit), or any page or the quit
question opening by another path, cancels the wait but keeps the latch: the release
would otherwise close what the player just opened, and the next Continue waits again
with a new deadline. The timed resume aid acts as Esc back to the column, then Continue. Completion is logged once as
`pause menu: resume after graphics change: pipelines waited <seconds> s, pending <n>, capped <true|false>`;
the existing `pause menu: resume` follows only when release is requested.

[design: `engine/nvforever/src/world.rs`, `loading_gate.rs`] Draw-distance streaming
stays outside this wait. Loading screens have cell/model readiness signals, but no
contract identifies which newly visible assets a distance edit must await. That part
of #1820 remains open; this change does not fully close the issue. Runtime compilation
timing and the two smoke setups remain unverified until run. State tests use synthetic
settings and controlled gate samples; smoke scenarios are `settings-resume-pipelines`
and `settings-resume-unchanged` in `tools/smoke/scenarios.toml`.

Pass order after the main pass (under FSR 3.1 the ambient occlusion comes before FSR, §3.1): tonemapping → ambient occlusion → temporal anti-aliasing, FXAA or
SMAA → sun shafts → bloom and sharpening → the game's image-space blur and grade (imagespace.rs) → UI. So the game's grade and its modifiers
(fades, blur, double vision) work on top of the improvements exactly as on the vanilla picture,
and nothing repeats them.

### 3.1 Ambient occlusion (`ambient-occlusion`)

From the depth prepass alone (multisampled or not: sample 0 is read): the view-space position
from the reverse-infinite depth (distance = near / depth), the normal from the neighbouring
depths on the side with the smaller step, 12 samples on a 3-turn spiral within 64 game units
(about 0.9 m; at most 80 pixels on the screen), turned per pixel by a 4 × 4 ordered pattern;
each sample occludes by its cosine to the normal above 0.12, fading out between half and the
whole radius. Strength 0.55. Since 2026-11-02 (#1370) this is worked out at half resolution,
for the top-left pixel of each 2 × 2 block, into a two-channel texture that keeps the pixel's
distance beside its openness (a quarter of the work; the pattern turns per half-resolution
texel). The result is blurred over the 4 × 4 texels of the pattern around each pixel, keeping
to texels within about 5 % of the pixel's own distance (so nothing bleeds across a depth edge),
and multiplied into the tonemapped picture. Those 4 × 4 texels span about 8 × 8 pixels of the
picture against 4 × 4 before, so contact shadows are a little softer. A pixel no nearby texel
shares a distance with (a pole or a grass blade a pixel wide) takes the texel nearest its
distance. Distances are held to 60,000 units (half floats end at 65,504; the sky dome is at
175,000 or more, far past the fade). It fades out between 4,000 and 7,000 units (where the radius is a pixel or two and the
fog takes over). The sky is left alone: the dome is opaque and in the depth prepass, but at
175,000 units or more, far beyond the fade (corrected 2026-10-12: not depth 0 as first written).

Known limits: it darkens sunlit creases too (a post-process cannot tell ambient from direct
light); water is not in the depth prepass, so the lake bed's occlusion shows faintly through
the water surface; the first-person arms take a little occlusion from their own folds.

While FSR 3.1 upscales (since 2026-11-08) the same pass runs before FSR instead, at FSR's
render size, multiplied into the HDR picture FSR then accumulates
([fsr-upscaling](fsr-upscaling.md), "Ambient occlusion"); `NVF_AO_BEFORE_FSR=0` keeps it
after tonemapping.

### 3.2 Longer sun shadows (`fine-shadows`)

**Since 2026-11-03 (#1367) the maps stay 2048²**; the switch keeps only the longer reach,
and the settings page calls it "Longer sun shadows" (the INI key stays `fine-shadows`).
The owner chose the frame rate: "we're going for the smaller one i want as much frames we can
get". [observed 2026-11-03, main 650911ee, i9-14900K, RTX 4070, 1920 × 1080,
`--graphics enhanced`, `--play` standing still, `NVFOREVER_HOME` without stored settings;
one run per place cycling the three sets every 10 s (`--graphics-cycle`), the first loop left
out, three or four periods each]

| Place | 4096², reach 12,000 (before) | 2048², reach 12,000 (now) | 2048², reach 8,000 (switch off) |
|---|---|---|---|
| Novac 07:00 | 11.30 ms | 9.02 ms | 8.70 ms |
| Novac 12:00 | 10.88 ms | 8.75 ms | 8.52 ms |
| Goodsprings 12:00 | 7.60 ms | 6.94 ms | 6.77 ms |
| Strip 22:00 (moon shadows, always 2048²) | 6.14 ms | 6.10 ms | 6.11 ms |

The shadow passes' GPU time at Novac 12:00 went from 1.82 to 0.87 ms with the smaller maps;
the rest of the difference was the render thread waiting for the GPU. Screenshots from the
free camera close to the ground (Goodsprings 12:00, Novac 07:00, two headings each, pitch
−35°) show the same shadows with slightly softer edges at 2048².

Before #1367: `DirectionalLightShadowMap` 4096² instead of 2048², and the last cascade at 1.5 ×
`--shadow-distance` (12,000 units by default since #254), the first cascade bound unchanged (a twelfth of
`--shadow-distance`): the nearest shadows get twice the texels per unit, the far cascades about
1.3 times, and shadows reach half as far again. Before #254 the arms' shade light copied the
cascades, so it also got 4096² maps: about 512 MB of shadow maps in all (vanilla: 128 MB).
Since #254 it has one cascade of its own (256 units); the total has not been measured again.

**By night and at the horizon** (#1369, owner 2026-11-02: cheaper moon shadows;
`graphics::sun_shadow_quality`): below the horizon the sun's light is the moon's (`sky.rs`),
and it casts one cascade out to 3,000 units (`MOON_SHADOW_DISTANCE`) in 2048² maps whatever
this switch says; while the light is under a tenth of its strength in its fade at the
horizon (`SHADOW_FADED`), no shadow maps are drawn. [observed 2026-11-02, i9-14900K, RTX 4070,
1920 × 1080, 8x MSAA, enhanced] With the day's 3 cascades at 4096² (before #1367) the moon's shadows cost
about 2.5 ms on the Strip at 22:00 (9.2 ms against 6.7 ms without sun shadows).
In interiors the sun is the cell's directional light (XCLL) and the clock stands still, so
it always casts the day's shadows there. The map size (`DirectionalLightShadowMap`) is one
setting for every directional light, so the first-person arms' shade light (`player_body.rs`
`shade_light`, its own 256-unit cascade) has 2048² maps too.
The phase is applied again when a new sun is spawned (a worldspace switch) and when
something else puts cascades on the sun (`sky::set_shadows`, the settings page).

### 3.3 Bloom (`bloom`)

On the tonemapped picture (linear values 0..1): the brightest channel over 0.82 with a soft knee
of 0.12 is kept at a quarter of the size (four bilinear taps over 4 × 4 pixels), blurred across
and down with a 9-tap Gaussian two texels a step (about 34 quarter-size texels wide, ~136 pixels
of the full picture), and added back at 0.9. This is how bloom was done in the original's era,
on display values; it does not touch the HDR parameters of the game's image spaces, which are
still not decoded (image-modifiers.md §5).

### 3.4 Sharpening (`sharpen`)

Contrast-adaptive against the four neighbours: the centre is pushed away from their sum by
0.5 × ¼, less where the neighbourhood already has contrast (none above 0.6), and held within
85 %-115 % of the neighbourhood's range so no bright or dark rims appear.

### 3.5 Anisotropic filtering 16x (`anisotropic`)

The game's textures (`nvf_assets::texture::sampler`) get 16x instead of 8x, and the default
sampler (generated LOD and other engine-made images) 16x instead of none. The switch applies to
textures loaded after it changes (cells loaded next); the start's textures follow the command
line.

### 3.6 Anti-aliasing (`anti-aliasing`; not part of `enhanced`)

One choice on the settings page (since 2026-10-16; before, temporal anti-aliasing was a switch
`taa` of its own and the multisampling only `--msaa`): **Off, 2x, 4x, 8x** (multisampling, the
start being `--msaa`, default 4x), **FXAA**, **SMAA** or **TAA**. The last three draw the view
without multisampling and smooth the picture after tonemapping, before the sun shafts, bloom and
the game's image space (the same place as TAA, so the grade is not smoothed). Choosing one of them
keeps the multisampling level for when a level is chosen again. Stored as a part in the INI like
any slider (`anti-aliasing=0.33` is 4x; the seven choices are the steps 0, 1/6, … 1). Unattended
runs keep `--msaa`, so `--graphics vanilla` draws as before.

- **FXAA**: Bevy's (`bevy::anti_alias::fxaa::Fxaa`) at its default thresholds (High, High)
  [engineering choice, not tuned by eye]. Cheapest; softens fine texture detail and does not
  smooth the plants' cut-outs (tested per pixel without multisampling).
- **SMAA**: Bevy's (`bevy::anti_alias::smaa::Smaa`) at its default preset (High) [engineering
  choice]. Sharper than FXAA, a little dearer; the same limit on the cut-outs.
- **TAA**: ours, as follows.

What the multisampling levels cost (#1379; the settings page's note gives the first two rows'
figures in its own words) [observed 2026-11-02: plain release build, i9-14900K, RTX 4070
(driver 616.56, Vulkan), 1920 × 1080, `--graphics enhanced`, vsync off (`--log-frame-times
10`), `--play` standing still, `NVFOREVER_HOME` without stored settings, `--msaa N`, 45 s runs]:

| Place | Off | 2x | 4x | 8x |
|---|---|---|---|---|
| Novac 12:00 | 10.36 ms | 11.19 | 11.63 | 12.76 |
| Goodsprings 12:00 | 7.15 ms | 7.92 | 8.45 | 9.37 |
| Strip 22:00 (the CPU sets the frame) | 6.26 ms | 6.23 | 6.30 | 6.31 |

So 2x adds about 0.8 ms, 4x about 1.3 ms and 8x about 2.3 ms where the GPU sets the frame.
The steps and the 4x default stay (#1379, a note only): the note tells a player turning
everything up what 8x costs and names FXAA, SMAA and TAA as the cheaper choices.

#### Temporal anti-aliasing

The other choice to multisampling, not on top of it: while it is on the main view is drawn
without multisampling (`Msaa::Off`); switching it off puts back `--msaa`. The alpha-tested
cut-outs (plants, fences, hair, grass) are then tested per pixel and smoothed by the jitter
over the frames instead of by alpha to coverage.

- The projection is shifted every frame by a sub-pixel amount (Halton 2,3, eight steps;
  Bevy's `TemporalJitter`), textures sampled at a mip bias of -0.5.
- After tonemapping and the ambient occlusion, before the sun shafts, bloom, sharpening and the
  game's image space (so the grade, fades and blurs are not smeared over the frames): each
  pixel's point is carried from this frame's camera to the last frame's (the depth and both
  cameras' matrices; the reprojection is worked out on the CPU in double precision, the world
  being hundreds of thousands of units across), taking the nearest depth of the 3 x 3
  neighbourhood so edges keep to the object in front. The history is sampled with a
  Catmull-Rom filter and clipped to the colours around the pixel this frame (variance clipping
  in YCoCg, 1.25 sigma, within the neighbourhood's min/max), then blended: 10 % of this frame
  standing still, up to 50 % when the picture moves 16 pixels a frame or more, weighted by
  inverse brightness. The history is a pair of Rgba16Float textures; it starts over on the
  first frame, after a resize, when the camera jumps more than 400 units in a frame (doors,
  MoveTo) and whenever the pass did not run the frame before.
- Bevy's own TAA needs motion vectors from every material's prepass; ours write depth only,
  so what moves by itself (actors, plants in the wind, water) is looked for where it was not;
  the clipping keeps that from smearing, at the cost of less smoothing on it while it moves.
- The first-person arms and weapon move with the camera: points nearer than 48 units are taken
  as not moving on the screen, and the neighbourhood's nearest depth is taken only among
  pixels on the same side of that distance (otherwise the world next to the hand took the
  hand's motion and left a dark rim while turning [observed 2026-10-12, fixed]). A wall nearer
  than 48 units counts as camera-locked too (it then keeps less smoothing while turning).
- Switching the multisampling while the game runs: in the frame it changes, the view's `Msaa`
  and its depth prepass texture can disagree; the passes that bind the depth (ambient
  occlusion, sun shafts, TAA, the image-space blur) now go by the texture's own sample count
  and skip that frame (before, a validation error ended the run).
- Looks [observed 2026-10-12, Goodsprings noon, standing, and walking while turning 40 degrees a
  second]: edges as smooth as MSAA 4x standing still, plants' cut-outs smoother; textures a
  little softer standing still and clearly softer while the view moves (the usual TAA trade);
  the arms stay sharp. **Not in `enhanced`** for that softness and because the owner plays with
  8x multisampling; the settings page offers it (`anti-aliasing` TAA).
- Cost: see §5.1; the pass itself ~0.05 ms here, and the main pass and the prepass are cheaper
  without multisampling, so TAA costs less than MSAA 4x.

### 3.7 Sun shafts (`light-shafts`)

Screen-space light scattering, at a quarter of the size, on the tonemapped picture after the
temporal anti-aliasing and before bloom: the sky around the sun (where the depth prepass has only
the sky dome, beyond 157,500 units) within 0.3 of the picture's height of the sun, its brighter
part (linear luminance 0.04 to 0.4), is blurred along the lines towards the sun (24 taps over
1/24 of the length, then 24 taps over 85 % of the way to the sun, each weighted 0.97 of the one
before: about 576 taps in all) and added to the picture, kept below white without a hard clip.

Tied to the original's look: the light is the game's own sky and sun as drawn (their colours,
the clouds in front of the sun), and how strongly it scatters follows the weather's fog:
strength `0.6 + 0.9 x haze`, `haze = min(1, 20,000 / fog far) x fog strength` (NVWastelandClear,
fog far 200,000: haze 0.1, strength 0.69; a weather with its fog far at 20,000 or less: 1.5).
None indoors, none once the sun is down (the sun disc's own fade, -2 to 3 degrees of elevation),
fading out as the sun leaves the screen (from the edge to 1.6 x the half-width beyond it).

Looks [observed 2026-10-12, Goodsprings 19:00 facing the sunset]: light streams past the
mountains and over the town, the sun's glow is wider; at noon looking away from the sun nothing
changes. Every constant is a choice by eye on that one view; the first try (strength 1.5, source
radius 0.45) washed the whole view out.

### 3.8 Screen-space water reflections (`water-reflections`)

The planar reflection (reflection.rs) shows one water plane at a time, the nearest below the
camera; water at another height in view reflects only the sky gradient. There, with this switch,
the water shader (water.wesl `ssr`) marches the reflected ray through the depth prepass (28 steps
from 24 units, each 1.16 times the one before, about 9,000 units in all, then four halvings) and
takes the opaque picture (the transmission copy) where the ray passes behind something by less
than a step; trust fades near the picture's edges and towards the end of the march. Mixed in
after fog and tonemapping, in the reflected share's place (an approximation: the copy is already
fogged and tonemapped).

Where it applies: exterior water at a second height in view, e.g. Lake Las Vegas (water 3,000)
next to Lake Mead's cells (2,600), Camp Forlorn Hope (5,360), the Devil's Throat (2,290)
[observed: fnvtool world cells, the cells' water heights in WastelandNV]. Interior cells draw no
WATR planes in this engine (the Goodsprings Cave pool is a model with its own material), so the
switch does nothing indoors. Checked only by forcing it in place of the planar reflection at Lake
Las Vegas (`--pos=28500,59000,3500 --heading 200 --pitch -12`, a temporary shader change): the
far shore's cliffs show in the water as with the planar reflection, without artefacts; that water
reflects weakly, so the difference is small. It runs only on the water pixels it applies to, so
its cost was not measured.

### 3.8.1 Mirrored water (`water-mirror`; on in both presets)

The planar reflection itself (reflection.rs): a second camera draws the world mirrored in the
nearest water plane while that water is on screen. It is on in vanilla too, since the original's
water reflects the world; `--graphics-only no-water-mirror` or the settings switch turns it off
(#1380), and water then reflects the sky gradient, and the picture where `water-reflections` is
on. Cost at Hoover Dam by day, Lake Mead in view, where the CPU sets the frame: 7.6 ms a frame
with it, 7.1 without [observed 2026-11-02, RTX 4070, 1920x1080, `enhanced`]. The camera keeps
its own set of the sun's cascades (Bevy's renderer needs them for every view the sun lights);
emptying their caster lists was tried and saved 0.12 ms of shadow-pass GPU time and nothing of
the frame, so the reflected world keeps its shadows.

### 3.9 Shadow filtering (`shadow-filter`; not part of `enhanced`)

How the sun's shadow maps are sampled at a shadow's edge, Bevy's `ShadowFilteringMethod` on the
main camera: **Fast** (`Hardware2x2`: one hardware-filtered tap, harder and more stair-stepped
edges), **Smooth** (`Gaussian`, Bevy's default and what vanilla has always drawn: the camera then
carries no component at all) or **Temporal** (a rotated 8-tap pattern that changes every frame:
soft but grainy unless TAA is the anti-aliasing, which it is meant for). Matters only with sun
shadows on. The names are engineering choices; Bevy's PCSS is not offered (it needs the
`experimental_pbr_pcss` feature).

### 3.10 Field of view, vertical sync, frame-rate limit (Display page)

Settings rows that are not improvements and do not change the vanilla picture unless chosen:

- **Field of view** (`field-of-view`): the game's FOV setting (the horizontal angle at 4:3, as
  `--fov`, default 75) from 50 to 110 in steps of 5, shown in degrees. It sets the player
  camera's projection at once; aiming down sights zooms from it (aim_zoom.rs takes a changed
  angle as its new base).
- **Viewmodel field of view** (`viewmodel-field-of-view`, under Field of view): the angle the
  first-person arms and weapon are drawn with (as `--viewmodel-fov`, degrees as `--fov` counts
  them, default 55, the INI's `fDefault1stPersonFOV` [documented: `Fallout_default.ini`]) from
  30 to 90 in steps of 5. `player_body` reads it each frame for the arms' magnification
  (`arms_magnification`), so it takes effect at once; the world's view is left alone, the
  Pip-Boy keeps its own 47, and the console's `fov` with a first-person angle still wins
  [design]. Scripts read both rows' angles, `settings::script_fov` telling them to the game
  state before the scripts run (`CameraRequests::settings_fov`): GetCameraFOV and
  GetPlayerCamFOV answer a SetCameraFOV (console `fov`) value first, then these, then the
  INI's; moving Field of view drops a console world angle, which the camera no longer shows
  [design].
- **Display sync** (`display-sync`, a dropdown; #1082): Vertical sync, on as Bevy's default
  (`PresentMode::AutoVsync`); Off is `AutoNoVsync`; Variable refresh (G-Sync, FreeSync) is
  vsync with the frame rate capped a few frames under the monitor's refresh rate (winit's
  current video mode; 144 Hz → 138). Kept as `display-sync=off|vsync|variable`; an INI with
  only the old `vsync=1`/`0` switch still sets it. Not on the page while `--log-frame-times`
  measures (which turns vsync off). What vendors document about VRR in borderless and
  exclusive full screen: spec-notes/display-sync.md.
- **Frame rate limit** (`frame-cap`): Off, 30, 60, 90, 120 or 144 frames a second
  (`frame_pace.rs`): each frame waits until 1/limit s has passed since the last (sleep, then
  spin the last millisecond), right after it is handed to the render thread and before the
  next frame's input is read, so the wait is not between the input and the picture (until
  2026-11-10 it waited at the end of the main world's frame, after the input; measured in
  [frame-generation](frame-generation.md), "Measurements"). A late frame is made up for a
  hundredth of a frame's time at a time. Bevy has no limiter of its own (`bevy_framepace` is a
  separate crate); this plain one is enough to keep a fast GPU from drawing hundreds of
  frames a second in menus [engineering choice].

### 3.11 Light glow (`glow`; on in vanilla too)

The original as the owner plays it (`bDoHighDynamicRange=1`) glows: neon, bulbs, lit
signs, fire and beams bleed soft halos into the night (the fx audit frames of 2026-10-23,
`C:\fnv-private\references\fx_audit\game`). The owner asked for that look, and for a
natural one rather than a copy, so this is on by default in `vanilla` and in `enhanced`;
`--graphics-only no-glow` or the settings switch "Light glow" turns it off, and the
enhanced "Bloom" (§3.3), when on, stands in for it.

- The main camera gets an HDR view (`bevy::camera::Hdr`: values over 1 survive until the
  tonemapper, which then runs as its own pass) and Bevy's bloom with its NATURAL preset
  (energy-conserving, no threshold: every pixel scatters a share of its light, so only
  what is far brighter than its surroundings shows a halo) (`post.rs` `glow_bloom`).
- Its strength is 0.15 at the image space's Bright Scale 2 (NVDefaultExterior) and
  follows Bright Scale / 2 (held to half and twice) as the place's image space and the
  weather's adapters change (`post.rs` `glow_intensity`, world.md §6 "HDR and bloom").
- What emits light is brighter than white only in this view: unlit shapes are drawn at
  their emissive colour × multiplier (nif.md §6 "Emissive multiplier of unlit shapes"),
  so Gomorrah's neon at multiplier 25 burns; lit shapes' emissive light already is.
- Every pass after the tonemapper (ambient occlusion, anti-aliasing, sun shafts, Bloom and
  sharpening, the image-space grade) sees the same tonemapped values as before; the
  passes key their pipelines on the view's format.

Engineering choices made by eye against the original's frames; not used to explain
differences in the parity frames.

### 3.12 Blend meshes into ground (`mesh-terrain-blend`; off in both looks, not part of `enhanced`)

Where a placed road, rock or cliff meets the landscape, the mesh's lower edge takes the
landscape's own colour and shading over a short height band, so a broken asphalt shoulder
reads as ground-lit dirt instead of a hard seam (#2661). Off by default and in `vanilla`,
left out of `enhanced` (false in both looks, so `quality.rs::enhanced_keys` leaves it out;
a test says so) until the maintainer decides; the settings row is under Graphics > Picture
after the enhanced look's rows, kept in the INI like them, `--graphics-only
mesh-terrain-blend` on the command line. Skyrim Community Shaders has a Terrain Blending
feature; the design is our own. Every band, angle, resolution and selection rule below is
NVForever's choice [design], not a fact about the original game or another renderer.

**The look the row draws** [design: `mesh_terrain_blend.rs::setting_config`]: the `height`
shape with an 8-unit band, 1024 colour texels per cell, full shading
(`8:surface:height:1024:full` in the test aid's terms). [observed: the main session's
round-7 comparison pictures at a WastelandNV road shoulder] `height` turned the broken
asphalt edge into ground-lit dirt and left the road top alone; the hybrid shapes either
removed the effect or striped the road's lip. Rocks stay subtle with it; that was accepted.

**Method** [design: `terrain_surface.rs`, `nvf-assets/src/nif_surface_blend.rs`,
`nif_surface_bindings.wesl`, `nif_material.wesl`]:

- *A world-space cache of the landscape around the camera.* A dedicated orthographic camera
  on its own render layer (16, `BAKE_LAYER`) draws flat, unlit copies of the 3 by 3 cells
  around the selected centre cell into one RGBA16F atlas of `3 x resolution` texels a side, one
  mip. Each quadrant's copies are its whole near-terrain stack in drawing order: the base
  (a folded base with its own `TerrainMaterial` in an unlit bake variant, any other base
  with the single-texture `SurfaceCaptureMaterial`) and every blended layer over it with
  its texture, vertex alpha and sort bias, composited by the GPU's own blend state. Nothing
  is lit, fogged or scorched; weather and light changes need no rebake. The selected centre
  is held separately from the committed front publication, with 256 game units of
  hysteresis per axis: retain coordinate `c` throughout the inclusive interval
  `[c*4096 - 256, (c+1)*4096 + 256]`, otherwise select `floor(position/4096)`.
  Backfill and cache assembly use this same selection, with southwest origin centre
  minus `(1,1)`; epoch/off transitions reset it. At `y=-8192`, a northward recentre needs
  `y > -7936`, and a southward return needs `y < -8448`. Teleports select directly.
- *Exact heights.* Beside the colour, a 576 by 99 RGBA8 atlas holds the f32 coefficients of
  both triangles of every LAND square (`height_planes`), packed losslessly into bytes and
  read with `textureLoad`, never filtered; the shader picks the triangle with the mesh's own
  diagonal and evaluates its plane, so the band follows the drawn landscape exactly. Each
  cell's tile ends in a metadata row: a nine-bit mask of the resident neighbours and a
  four-bit mask of the quadrants whose colour was captured. Every prepared cell gives its
  heights even where a quadrant's colour cannot be captured.
- *Residency.* Before sampling, the shader maps all four bilinear taps to their cells and
  quadrants and needs every height and colour bit (a missing diagonal cell of an L-shaped
  region included); a 128-unit fade at the region's edges remains. A failed lookup leaves
  the fragment exactly as it was.
- *Publication and continuity.* Each immutable front/back publication owns its images,
  southwest origin and cell identities. A camera crossing retains completed, resident,
  unchanged cells inside the new 3 by 3 region at the **old origin** until replacement
  commits. An enabled-only nine-bit coverage mask intersects baked height and quadrant
  validity in the initial lookup, every bilinear footprint tap and each border-fade
  neighbour check. Departing or dirty cells lose coverage without a colour-image upload.
  Continuity covers retained interior footprints; new cells, genuinely invalidated cells,
  teleports without overlap and changing missing-neighbour border fades have no guarantee.
- *Incremental assembly and receipts.* Unchanged cells are GPU rectangle copies into a
  fresh back atlas; only entering/dirty cells have flat draw copies. Fully populated axial
  and diagonal crossings copy/bake 6/3 and 4/5 cells; one changed cell copies/bakes 8/1.
  Removal alone changes coverage; unchanged frames allocate no atlas and do no copy/bake.
  The small height/metadata image is rebuilt for a replacement. Copies use the bake's
  reversed Y and run in render cleanup **after** `render_system` has submitted all final
  camera-target writes (`engine/vendor/bevy_render/src/lib.rs`), in a following queue
  submission. Exact dirty-draw receipts reset every frame; partial frames cannot accumulate.
  Copy-only work also needs a transfer receipt and both GPU images. Completed views leave
  the sorted cameras before another clear, and publication retains the extra main-frame
  delay. Receipts name epoch, request, destination and source revision; the current cell
  identities and source revision are revalidated before commit. No pixel readback or GPU wait.
- *The blend.* With `d` the fragment's height above the ground triangle, `B` the band and `F`
  the residency fade, the `height` shape's weight is `w = F (1 - smoothstep(0, B, max(d, 0)))`.
  Where `w > 0` the forward fragment, after the existing coverage tests and normal mapping
  and before lighting, mixes the base colour towards the cached colour by `w`; full shading
  also bends the normal towards the ground triangle's geometric normal and moves roughness
  towards the landscape's 0.95, reflectance towards 0.2 and metallic towards 0, and fades
  emission, clear coat and anisotropy by `1 - w`. Alpha, discard, coverage, positions, depth,
  the prepass and the geometric normal used for shadow bias are untouched; `w <= 0` leaves
  every input exactly as it was.
- *The landscape's own layers near the camera (round 10a)* [design: `terrain.rs`
  `terrain_draws`, `terrain_surface.rs` `DetailPalette`, `encode_detail`;
  `nif_surface_blend.rs` `NifSurfaceBlend`, `DetailQuadrant`; `nif_surface_bindings.wesl`
  `mesh_terrain_detail`]. [observed: the user's play-test at 1920x1080 near a road edge,
  round 9b] the colour cache alone reads as a smooth, slightly hazy band: it holds 4 game
  units per texel at 1024, while the ground beside it is sharp gravel with normal maps.
  So, within 4096 units of the camera, a contact fragment also reads the near landscape's
  own layer textures at its game xy and composes them as the near draw does. *Capture*:
  while the blend is on, each quadrant's capture also records what its near draw is made
  of, from the very meshes and materials drawn (after `stitch_layers`, so stitched
  neighbour layers are included): the base draw's source (the folded `TerrainMaterial` or
  the single base material), the separately drawn overflow layers' materials in drawing
  order, the base mesh's per-vertex linear colour, normal and tangent with the tangents'
  handedness, and one opacity grid per drawn overlay (the folded ones, then the overflow
  ones). Source material edits re-read it (`refresh_captures`); a source that is gone or
  no longer matches the prepared layers leaves that quadrant on the colour cache. Its
  images (normal maps included) join the cell's dependencies. *Publication*: with the
  capability, each publication's height atlas gains 201 rows (576 by 300 RGBA8 instead of
  576 by 99; the bytes keep their 32-bit words, read with `textureLoad`): a 32-word entry
  per quadrant (flags with the folded and total overlay counts, palette numbers of the
  base's diffuse and normal map and of each overlay's, the base diffuse and normal
  samplers, normal decoding bits, the base material's colour and texture-coordinate
  transform, the repeat, whether the coordinate is cell-local as in a six-layer folded
  draw, the tangents' handedness), then 11 words per vertex of the quadrant's 17 by 17
  grid (linear colour as unorm16, normal and tangent as snorm16, up to 12 opacities as
  unorm16). A third texel in each cell's metadata row holds the four-bit mask of the
  quadrants whose detail it supplies. The palette (`TerrainPalette`) lists the distinct
  layer images and sampler descriptions (`ImageSampler` equality; texture.rs gives the
  landscape textures one description) of the admitted quadrants; every replacement repacks
  its own palette numbers. Quadrants are admitted whole, in a deterministic order (those the
  previous publication admitted for unchanged cells first, then by grid and quadrant); a
  quadrant that would overflow the palette, an image not loaded or an unresolved material
  keeps the colour cache, with its reason on the publication line (round 10a review: also
  `sampler-mismatch` and `overflow-material`, below). *Binding*: the blend
  type's bindless index table (binding 100) is 50..454 instead of 50..54: after the four
  ordinary entries come 384 texture and 16 sampler entries, registered through the blend
  material's own allocator (bindless slots are local to a material type's slab) and
  resolved through the clone's own table row; unused entries hold the fallback image. No
  binding is added, bindless or not, and without bindless the type's layout is exactly the
  colour interface's (uniform 50, textures 51 and 53, sampler 52). Every clone of a
  publication binds that publication's palette, restricted to its live coverage (round 10a
  review, `PublishedSurface::live_palette`): when cells are masked (a departing or changed
  cell, or one whose image was removed: a removal bumps the image's dependency revision
  and so masks every cell reading it in the same update), the entries only masked cells'
  quadrants read become the fallback, keeping their numbers (the immutable height atlas's
  numbers keep naming the same entries), and a sampler still sampled takes as its
  representative an image a covered quadrant reads with that description (second review
  round: a quadrant samples its diffuse sampler only with a base diffuse or an overlay
  and its normal sampler only with a base normal map, so a textureless quadrant keeps no
  sampler alive; a sampler without a live representative binds the fallback sampler,
  never the publication's original representative, which may be the removed image). The coverage
  update rebuilds retained clones (Bevy drops the old prepared material before preparing
  the new one), and new clones of the front take the same restricted palette, so neither
  can wait for an image that is gone and the retained clones keep drawing. The masked
  cell's replacement bake cannot wait for one either (second review round): a folded
  base's bake copy binds no normal map (`terrain_surface.rs::bake_copy`; the unlit bake
  returns the base colour only), and a cell whose bake copies would bind a colour image
  removed from the image assets (and not added back) is planned without that cell,
  excluded as `missing-colour-image` (an image not loaded yet is still waited for); it
  bakes again once the image is added back. A detail image that returns (added back after a
  removal) or arrives late (a replacement planned without it refused its quadrants,
  `image-not-loaded`) bumps its dependency revision too (third review round), so the
  resident cell's request changes and the quadrant's detail is rebuilt instead of staying
  on the colour cache; only the first of its Added / LoadedWithDependencies notifications
  does, so one return makes one replacement. *Composition* (`mesh_terrain_detail`): the
  fragment's ground triangle and its barycentric weights come from the height lookup's own
  square and diagonal (below the diagonal `(1-u) SW + (u-v) SE + v NE`, above it
  `(1-v) SW + u NE + (v-u) NW`); the colour starts as the base diffuse x interpolated vertex
  colour x base material colour, and each overlay in drawing order mixes in by
  `saturate(opacity) x` its diffuse alpha (ordered opacities, not normalized weights), its
  colour the texel x vertex colour x the base material colour for folded overlays (as
  terrain.wesl) and x white for overflow ones (their own material's colour:
  `layer_material` draws only a layer with a texture, white). With full shading the
  normal starts as the base's mapped normal (or the plain interpolated one), Bevy's own
  `calculate_tbn_mikktspace` and `apply_normal_mapping` on the interpolated, not
  renormalized, normal and tangent, the normal maps decoded with green flipped and each
  map's two-component flag (folded overlays with the base's, as terrain.wesl; overflow
  ones with their own), mixed towards each overlay's mapped normal (or the plain one) by
  the same shares and normalized once. Texture reads use explicit gradients
  (`textureSampleGrad`): the derivatives of the fragment's game xy are taken in
  `nif_material.wesl` at the function's top level, before the lookup's branches, then
  mapped through `1 / repeat`, the base's texture transform and the view's mip bias; the
  base diffuse sampler reads every diffuse and overlay normal map, the base normal sampler
  the base normal map (terrain.wesl's rule for the folded overlays). *Samplers the detail
  does not reproduce* (round 10a review; the palette format keeps one diffuse and one
  normal sampler per quadrant): a textureless base binds Bevy's fallback image and so its
  fallback sampler, which reads the folded overlays in the near draw and need not equal
  any layer image's description (with anisotropic filtering off the configured fallback is
  1x, while texture.rs gives landscape textures their own description); separately drawn
  overflow layers read their diffuse and normal maps with those images' own samplers.
  So a quadrant with folded overlays on a textureless base, or with an overflow layer whose
  diffuse or normal image's description differs from the detail's diffuse sampler (the
  base diffuse's, or the first overlay's without a base texture), keeps the colour cache,
  logged as `sampler-mismatch` (`terrain_surface.rs::samplers_reproduced`). Toggling
  anisotropic filtering between texture loads can make such descriptions differ in
  ordinary play. The target is never taken from the mesh pixel's
  screen-space prepass normal. *Sources and distance*: colour validity (the quadrant and
  the whole bilinear footprint) and detail validity (the quadrant's detail bit, live
  coverage, nearer than 4096 units) are checked separately, so a valid detail lookup never
  fails for an unused colour tap, and the `status` view shows success where either source
  serves the fragment. With `d = 1 - smoothstep(2048, 4096, distance)`: both valid, the
  target colour is `mix(cache, layers, d)` and the target normal
  `normalize(mix(geometric, mapped, d))`; detail alone, the layers with the weight faded by
  `d`; colour alone, the cache and the geometric normal; neither, no blend. The contact
  weight and the residency fade are unchanged. *Overflow layers and the cap* (NVForever's
  choice, round 10a): [observed: the main session's publication lines at the user's test
  road] cell (-17, 0) publishes quadrants `folded+1-layers, folded+2-layers, folded,
  folded+1-layers` and cell (-17, -1) `folded, folded+1-layers, folded, folded+2-layers`:
  road cells routinely have overlays beyond the six that fold, because `stitch_layers`
  adds neighbour textures, so sending such quadrants back to the colour cache would leave
  exactly the user's spot blurry. The detail therefore also composes the separately drawn
  overflow overlays, after the folded ones and in their drawing order, up to 12 overlays
  plus the base (12 opacities fit three vec4s; a vertex record is 11 words with the colour,
  normal and tangent); a quadrant with more keeps the colour cache, logged as
  `overlays-over-cap`. The detail composes every overflow layer with white at full alpha
  through the base's texture transform; the near draw uses each layer's own material
  colour, alpha and transform (`layer_material`: white, opaque, the default transform). A
  quadrant whose overflow layer's material has another colour or alpha, or another
  transform than the base's (a base transform edited alone included), keeps the colour
  cache, logged as `overflow-material` (round 10a review,
  `ResolvedDetail::from_materials`); the check runs at capture (turning the blend on, a
  backfilled cell, a cell drawn while it is on) and again at every source material refresh
  (`refresh_captures`), and both keep the capture with the reason (second review round:
  the capture too, `terrain.rs::DetailParts::capture`), so an edit back serves layer
  detail again. This is an approximation: the near terrain alpha-blends each
  overflow layer's *lit* result, while the detail composes albedo and normal before
  lighting once. [hypothesis] The difference is small because the layers share the
  surface's lighting and differ only in their mapped normals. Every match described here
  is of prepared triangle attributes, texture mapping, ordered surface composition and the
  normal-map convention for the supported quadrants; it does not promise identical final
  pixels (filtering footprints on differently oriented mesh geometry, lighting position,
  shadows, occlusion, the near landscape's prepass normals, scorch and coverage still
  differ) and does not remove geometric silhouettes. *Budget*: 384 images and 16 sampler
  descriptions per publication, so one material's resources (its standard textures, the
  two atlases, the palette) stay inside the 512-resource bindless slab; a quadrant reads at
  most 26 images (13 diffuse, 13 normal), so at least 14 entirely distinct quadrants fit,
  and quadrants sharing LTEX textures share entries. *Capability*: the blend material
  bindless on the device and non-uniform indexing of sampled-texture arrays
  (`mesh_terrain_blend.rs::detail_capability`); otherwise, or with the aid below,
  publications carry no detail rows and no palette and the colour cache alone serves, as
  before. Nothing of this runs while the blend is off (no capture, no rows, no palette).

**Selection** [design: `world.rs::spawn_model`, `blend_census`;
`mesh_terrain_blend.rs::reference_reasons`, `material_reasons`]: rigid exterior STAT and
SCOL shapes. A SCOL is a static collection: "composed of Static forms", "do not react to
Havok physics and cannot be destroyed", its Model "the NIF file used for the object", which
the editor generates when the collection is made [documented: GECK wiki "Static
Collection", revision 40901, and "Bethsoft Tutorial Static Collections", revision 38886,
offline snapshot 2026-10-28]. `world.rs` draws a SCOL reference as that one model, as it
draws a STAT, so the same reference, shape and material rules apply to both. A reference is
left out for any of: not in an exterior cell of the drawn world, a rig or skinning,
particles, jointed or loose bodies or a stored pose, damage-stage nodes, a DEST record, an
imposter; a shape for external emittance or a registered morph mesh. A material is left out
for Add, Multiply, Premultiplied or AlphaToCoverage alpha, unlit, refraction, first person,
depth write, transmission, falloff, holes, FaceGen, the Pip-Boy screen, linear add, soft
edges or a non-finite depth bias; ordinary Blend, a finite depth bias, the lit decal flag,
Mask and an alpha cut are admitted and keep their descriptors exactly. A material owned for
the moment by a fade, an effect shader or a material override never blends; the owner's
restore brings the blend back. Never by asset name.

**Excluded altogether**: interiors, distant LOD and imposters, actors and creatures,
first-person and interface meshes, movable clutter (MSTT, items, posed clutter), doors,
activators, trees and grass, water.

**Materials and switching** [design: `mesh_terrain_blend.rs`]: an eligible shape draws with
a separate material type, `ExtendedMaterial<StandardMaterial, NifSurfaceBlend>`, a clone of
its source shared per source handle and visible publication/configuration; the ordinary
`NifSurface` interface is
unchanged (its 128-byte uniform, bindings 50-53), and the blend type binds no more textures
than the ordinary pipeline it replaces (the atlases take the holes and FaceGen slots, which a
blend clone never has). A source edited under the same handle (blended cut-outs flipping
Blend and Mask, scripted NIF edits) rebuilds its clone in the same frame. Ordinary-component
change ticks are tracked even before a shape's first clone is ready, so an emissive edit
and reinsertion evicts the pending candidate and its later acknowledgement cannot enable it.
Texture swaps, fades, draw-distance ranges, door fades, effect shaders and shadow thinning
follow the source. Replacement clones are prepared and acknowledged with a bound render material before
existing shapes switch; their old enabled handles and image references stay alive meanwhile.
Request restarts leave enabled clones alone. Coverage changes on already drawable clones
change uniform POD only, at each clone's own publication origin; the colour image stays
immutable. A fresh back allocation waits for the previous publication's material switches;
a cancelled job has a three-main-frame retirement interval before another allocation.
Transfer plans hold strong image handles and cancellation drops their main-world references;
extracted views and material assets release theirs through Bevy's ordinary retirement.
The row switches the blend while the game runs: on, it opens the material gate
(below), logs its start line, checks the atlas against the device, records the census of
the shapes already drawn and the captures of the cells already drawn around the camera
(their landscape prepared again, the same quadrant stacks and cached materials as a cell
drawn with the blend on), and bakes; off, every converted shape leaves the blend material,
and the census, the captures, the atlas and the clones are dropped, with one `stop` line.
The restoration (`stop`) runs in PostUpdate where the conversion runs, after the Update
producers (texture swaps, fades, effect shaders, overrides) and the PostUpdate ones (the
NIF mirror, the blended cut-outs) with their commands applied: an ordinary material a
producer put on the shape that frame stays (a texture swap's copy, whose generation the
swap has consumed; a fade's or an override's copy), and only a shape with none gets its
recorded source back.

**Off** [design: `mesh_terrain_blend.rs::BlendGate`, `close_gate`;
`terrain_surface.rs::SurfaceBakePlugin`; vendored bevy_pbr `MaterialGate` and bevy_render
`ErasedRenderAssetPlugin::gated`, engine/vendor/README.md]: both material types, the
clones' `NifBlendMaterial` and the bake's `SurfaceCaptureMaterial`, are registered behind
one gate, closed in a run that never turns the blend on. While it is closed none of their
own systems run (main world: change marking and the specialization check; render world:
asset extraction and preparation, instance extraction and sweep, specialization
extraction), their shaders are not loaded, no bind group allocator and no bindless
fallback buffer exists for them, the bake view is not extracted, the draw functions are
Bevy's own, and the receipt and span systems do not run (they need a bake view in the
render world). Nothing is captured, baked, allocated or logged. What still runs that main
does not have, every frame: `follow_setting` (the aid and the row compared with what is
drawn); the run conditions themselves, an atomic load for each gated system or group, a
resource check for each material type's opening and closing and for putting the draw
functions back, and an empty-query check for the receipt and span systems (the span
systems' once per view); and bevy_asset's tracking systems of the two asset types, whose
stores exist, empty (an asset type cannot be registered after startup). After the blend is
turned off, the gate stays open until no material of either type, as an asset or on an
entity, has been left for 3 frames (their removals extracted and unloaded); then the
render world drops both allocators and shader handles, puts Bevy's own `DrawMaterial`
command back in the opaque and transparent phases, and an `idle` line is logged, so the
run is idle as above again. Pipelines compiled while it was on stay in Bevy's pipeline
cache, as every pipeline does (Bevy never evicts them).

**Cache and invalidation rules** [design: `terrain_surface.rs`, `terrain.rs`]: the 3 by 3
region follows the camera; cell identity includes session/worldspace epoch, grid, resident
entity incarnation, capture content revision, component change tick and the revisions of
its captured diffuse-image dependencies. Residency is confirmed against `Streamer.loaded`.
Terrain reconstruction replaces the capture with a new revision; reconstruction of an
already captured resident input also makes its cardinal
neighbours reprepare stitched layer inputs, including loaded captures outside the selected
region, retaining their primary revision so this cannot recursively dirty the whole world.
Failed refreshes remain pending by entity incarnation within the cache context, are excluded
from coverage/copy/bake until refreshed, and clear on epoch/off transitions or unload.
Component replacement/in-place capture edits and modifications/removal of dependent
diffuse images invalidate the affected cells.
Capture materials retain their source handles: source-material changes refresh the actual
unlit clones and their diffuse dependencies before replanning, rather than rebaking stale
copies. Source-material removal makes that capture height-only until reconstruction.
This supports these engine reconstruction/material/image routes; arbitrary live
LAND record editing, near-mesh mutation outside reconstruction and load-order editing are
not implemented by this cache. Weather/light changes need no rebake. Worldspace/session,
interior, off and incompatible-resolution transitions cancel jobs and clear publications
before receipts are accepted, even without a camera. Request IDs and visible publication
revisions are separate from that epoch. Source asset events are consumed after
`AssetEventSystems` and before receipt acceptance; conversion uses the result next frame.
The ready line's historical `generation` field now names the visible publication revision.

Colour resolution remains 512, 1024 (default) or 2048 per cell side, checked against the
device's texture limit and refused when it does not fit. One RGBA16F atlas is 18, 72 or
288 MiB; front plus back at 1024 is **144 MiB of colour**, before CPU image copies, HDR
render intermediates and retiring/extracted handles. There is one active replacement,
fresh destinations rather than recycling referenced images, readiness admission and
cancelled-job retirement; rapid replanning cannot enqueue an unbounded allocation list.
Readiness admission waits for the front's materials to settle, but never longer than 1800
frames in a row (round 10a review, `SETTLE_FRAMES`): a candidate that can never be prepared
cannot hold every later publication back; the replacement then starts with a
`settlement wait expired revision=... frames=...` warning, so at most one allocation per
1800 frames comes that way.
There are still no gutters or mips in the colour cache (round 10b); beyond 4096 units, and
where a quadrant keeps the colour cache, geometric triangle normals remain the target.
Round 10a's detail adds about 0.66 MiB of height atlas per publication (1.3 MiB front and
back), 1600 bytes of index table per enabled clone and the palette's descriptor entries;
the layer images are the landscape's own, not copied. Scorch is excluded: capture has no
burn-map binding or synchronization, so full shading still differs through scorch,
separately lit overflow layers (composed before lighting, above), shadows/occlusion and
mesh geometry/coverage. The row stays height-8.

**Test aid** `NVF_PLAY_TEST_MESH_BLEND=[band:]view[:shape[:resolution[:shading[:flat,steep]]]]`
overrides the row for comparison runs and is not stored; a malformed value is refused in
the log and the row decides. Omitted parts take the row's look.

| Part | Values | Default |
| --- | --- | --- |
| band | positive finite number | 8 |
| view | `surface`; diagnostics `weight` (blue 0 to red 1, unlit), `normal` (the ground triangle's game-space normal as `0.5 + 0.5 n`, unlit), `selection` (a lit green tint on converted shapes), `status` (unlit: magenta outside the region, red no height, orange the quadrant's colour not captured, cyan a bilinear tap rejected, green success brightening with the weight) | (required) |
| shape | `height`; aid-only `penetration` (`1 - smoothstep(-2, 0, d)`), `hybrid` (v2: the band widened with the angle between the unperturbed mesh normal and the ground, from the flat width parallel to the ground ramping to the band at 5 degrees, the band from 5 to 35 degrees, ramping to the steep width at 75 degrees; the larger of that height band and the penetration term), `hybrid-v1` (round 5's orientation-gated band) | `height` |
| resolution | `512`, `1024`, `2048` | `1024` |
| shading | `albedo` (colour only), `full` | `full` |
| flat,steep | hybrid's widths, `flat <= band <= steep`; only with `hybrid` | `band / 16`, `6 band` |

Old short forms keep working with these defaults (`24:status` is band 24, `height`, 1024,
full). `NVF_PLAY_TEST_MESH_BLEND_DETAIL=colour` (round 10a) keeps the colour cache alone for
comparisons (any other value is refused in the log). Log lines: `mesh terrain blend: start
band=... variant=... resolution=... noise=0 shape=... shading=...` then ` flat=... steep=...
weight-version=2` for `hybrid` or ` weight-version=1`, then round 10a's resolved detail mode,
` detail=layers` or ` detail=colour detail-reason=aid|device|no-device`; `colour atlas resolution=... atlas=WxW rgba16f colour-mib=...`;
`backfill tracked-shapes=... convertible-shapes=...` as it turns on; an
`assembly epoch=... request=... copied=... baked=... retained-origin=... destination-origin=...`
line per replacement and one `publication ...` line per commit (resident grids with each
quadrant's capture path and colour mask, the
excluded slots, a probe of the uploaded heights under the camera with its CPU lookup
status (the colour cache's classification), `probe-normal` and `probe-detail-mask`, the
image IDs, the resolution, then round 10a's ` detail=layers|colour palette-images=N
palette-samplers=N detail-quadrants=N colour-quadrants=[(x, y):qN:reason;...]` naming each
quadrant that keeps the colour cache with its reason, or ` detail=colour
detail-reason=capability` without the capability); `ready eligible-shapes=...
resident-cells=... generation=... variant=... shape=... resolution=... shading=...`, the
weight fields, then the census (`stat-shapes`, `excluded-shapes`,
`pending-material-shapes`, one `excluded-<reason>` count per reason, `admitted-blend`,
`-bias`, `-decal`, `-mask`, `-alpha-cut`, then `scol-shapes`) and round 10a's ` detail=layers
detail-quadrants=N` (the publication supplies layer detail) or ` detail=colour`, printed at
each publication
and refreshed at most once per 60 frames; `census-shape ref=... base=... model=... shape=...
status=eligible|excluded first-reason=... reasons=... base-type=STAT|SCOL`, one per distinct
base, shape and reason mask, at most 2048 a run; `stop restored-shapes=... tracked-shapes=...
released-cells=... kept-producer-materials=...` (`restored-shapes` counts every converted
shape back on an ordinary material, `kept-producer-materials` those that kept one a
producer put on that frame); `idle materials-released=true` when the gate closes. A road without a `census-shape` line need not be untracked: the lines
stop at 2048, a shape whose material is still loading waits for its line, and a line is
printed once per base, shape and mask with the first reference seen, so a search by
reference can miss it; the ready line's counts cover every resident shape.

**Continuity test aid** [design: `terrain_surface.rs::continuity_probe`]:
`NVF_PLAY_TEST_MESH_BLEND_CONTINUITY=start-seconds:duration-seconds` (`--play` only, on
the same world-aid clock as LOOK/MOVE/SHOTS, `nvf-play/src/pause.rs`) arms only after an
initial publication and prepared shape witnesses in the starting camera cell; duration is
bounded to 120 seconds and 20,000 counted frames. It counts every subsequent main frame,
checks retained unchanged witnesses using their actual enabled clone's origin, height and
coverage, and checks render preparation acknowledgements. Frame lines are
`mesh terrain blend: continuity frame=N camera-cell=... transition=BOOL pending=BOOL
published=BOOL retained=N missing-enabled=N missing-prepared=N failures=N copied=N baked=N`.
The final `continuity summary frames=N crossings=N pending-frames=N retained-lookups=N
pending-retained-lookups=N unwitnessed-pending-frames=N replacements=N published-false=N
missing-enabled=N missing-prepared=N failures=N copied=N baked=N settled=BOOL recentres=N
pending-reversals=N` totals the full interval. Recentres count selected-centre changes;
pending reversals count direction reversals when work was pending in the preceding probed
frame. Pending includes atlas work, retirement and
material readiness; settled requires the final publication's material switches to finish. `NVF_PLAY_TEST_MESH_BLEND_READY_DELAY=N` delays each
new material's acknowledgement by at most 120 render frames for deterministic pending work;
it changes no appearance controls. These CPU/binding checks do not prove rendered pixels;
`mesh-terrain-blend-crossing` also captures before/during/after/settled frames for review.
`NVF_PLAY_TEST_MESH_BLEND_CROSSING=<y>:<z>:<delay>/<x>[,...]` (round 10a, after the round-9b
review: the go-to aid ran on the aids' elapsed clock and its 0.1-second pairs could fall in
one frame) lets the probe drive the crossing on its own world clock after arming: a step
puts the player's feet at game `(x, y, z)` `delay` world seconds after the previous step
(after arming for the first), and each later step also waits until the probe has seen the
previous step's recentre in a later frame with work pending in the current one, so every
reversal lands on a distinct later frame while a replacement is pending; a step still
waiting 10 world seconds after it was due goes anyway and counts as a timeout. Round 10a
review: a step's replacement could be prepared and settle (the 120-frame readiness delay
is one second at 120 frames per second) before a step with a 3-second delay came due,
which would then wait for pending work that never came, up to its timeout. So, while
steps remain, the blend materials first seen after a step are held unacknowledged until
the driver issues the next step (`BlendReadiness::hold`, `mesh_terrain_blend.rs::acknowledgeable`):
the step's replacement stays pending until then, whatever the next delay. Materials seen
before a step (the retained clones among them) are never held, and nothing is held after
the last step or the probe's summary; without the aid nothing is held. Lines:
`continuity crossing step=N x=... y=... frame=N world-secs=... gated=BOOL pending=BOOL
timeouts=N`, then `continuity crossing done steps=N timeouts=N`. The staging go-to before
arming stays with `NVF_PLAY_TEST_GOTO`. Its rapid reversals move 384 units to either side
of the boundary, beyond both hysteresis thresholds, with delayed material readiness; the summary requires at least four crossings
and recentres, two reversals during pending work, and final settlement. Synthetic tests
also count live images, meshes and capture materials across repeated cancellations and
completion using Bevy asset retirement over real app updates (not just request replacement).
Its boundary road-contact location remains a hypothesis until the main session runs it.

**Cost**: GPU cost of round 9 is unmeasured (checks only, no game run). The bake is timed
as the GPU span
`nvf_mesh_terrain_bake` (Tracy's GPU zones and the `--log-frame-times` list,
`terrain_surface.rs::bake_span`), from before the bake view's first pass to after its
last. Its two timestamps go into command encoders of their own, queued at the two systems'
places in the render order, so the span pairs the same with `NVF_PLAY_TEST_PROFILE_ENCODERS=1`
(whose automatic `enc:<system>` span opens on a render context's own encoder) as without
it. Retained rectangle transfers are submitted after that camera span and are not included
in it. The lookup in the meshes' fragments shows in the main opaque pass instead.

**Limits** [hypothesis where marked]: beyond 4096 units, and in quadrants that keep it, the
colour cache may soften contact detail, and a footprint turning invalid at a missing corner
can make a narrow transition [hypothesis]; there geometric triangle normals, not the
landscape's normal maps, are the shading target, so faceting can show [hypothesis]; a
quadrant on the colour cache beside one with layer detail can show a boundary
[hypothesis], including the quadrants refused as `sampler-mismatch` or `overflow-material`
(round 10a review: the palette carries neither per-layer samplers nor the overflow
layers' material colour, alpha and transform, so those quadrants are refused rather than
approximated); the detail's distance thresholds are initial choices for play-test review,
and its GPU cost (texture bandwidth, descriptor entries) is unmeasured; the visual
acceptance (gravel frequency and mapped-normal response at the close road shoulder at
1920x1080, grazing views, layer boundaries, the 2048-4096 transition, publication
crossings) needs the main session's frames, which logs and naga do not prove; shadows, occlusion and the separately lit layer draws still differ from
the landscape's own lighting; admitted Blend and decal shapes near the ground (glass,
markings) take the landscape's colour; buried visible surfaces take its look; the census
keeps scanning every tracked shape each frame while the blend is on (only its line is
limited); rigged STATs and SCOLs drawn while the blend was off are not counted until their
cell is drawn again (they never convert); the colour-coded `excluded` view was not built.

**Tests and smoke**: `nvforever` test filters `mesh_terrain_blend` (switching off during a
texture swap, a fade and an override: `switching_off_during`), `terrain_surface` (round 10a:
`palette_admits_whole_quadrants_in_order_and_refuses_overflow`,
`colour_only_publications_carry_no_detail`,
`crossing_steps_wait_for_the_seen_recentre_and_pending_work`; round 10a review:
`delayed_steps_find_the_previous_work_still_pending_after_it_prepared`,
`samplers_the_detail_does_not_reproduce_keep_the_colour_cache`,
`overflow_material_edits_keep_the_colour_cache_until_reproducible_again`,
`settlement_guard_waits_but_never_forever`; second review round:
`a_textureless_survivor_never_restores_a_removed_sampler_representative`,
`removed_images_never_leave_the_replacement_waiting_on_an_unpreparable_draw`; third review round:
`a_normal_map_added_back_restores_the_detail_once`,
`a_normal_map_loaded_late_ends_up_with_detail`), `mesh_terrain_blend` (round 10a review:
`removed_palette_image_keeps_retained_clones_drawable_and_the_replacement_publishes`),
`terrain::tests` (round 10a: `detail_capture_reconstructs_the_drawn_triangles`; second
review round: `an_initially_rejected_overflow_capture_recovers_when_the_edit_is_reverted`),
`mesh_blend_shader_tests` (round 10a:
`shader_detail_transcribes_the_mesh_triangles_and_the_record_layout`), `graphics::tests::mesh_terrain_blend`,
`settings::tests::the_mesh_blend_row`, `texture_swap`, `nif_mirror`, `fade`,
`draw_distance`, `door_fade`, `effect_shaders`, `shadow_thin`; `nvf-assets`
`nif_surface_blend::tests` (round 10a: `detail_layout_fits_the_height_atlas_and_round_trips`,
`detail_quadrant_words_name_folded_and_overflow_overlays`), `nif_material::tests`;
`nvf-play` `settings_layout`, `fade`. Smoke: the `mesh-terrain-blend-*` scenarios, round 10a's
`mesh-terrain-blend-detail-colour` (the aid's colour fallback) among them.
The WGSL is composed and validated by naga in the tests, not executed. Smoke:
`mesh-terrain-blend-crossing` (retained witnesses, pending work, replacement and zero
publication/material/lookup outages), `mesh-terrain-blend-setting` (the row on from the
command line), `mesh-terrain-blend-live`
(on and off from the settings page, then the gate closing), the aid's `mesh-terrain-blend-goodsprings`,
`-options` and `-widths`; `graphics-enhanced` and `settings-enhanced-effects` check that
the enhanced look neither turns it on nor logs it.

## 4. Tried and dropped

- **Bevy `ContactShadows`** on the sun (16 steps, 40 units, thickness 12): the right effect on
  sunlit bushes, but a band of dark speckles on near surfaces seen at a grazing angle (the
  near roof at Goodsprings) and per-frame noise, which Bevy expects TAA to clean up.
- **Fog in-scattering towards the sun**: invisible in NVWastelandClear (fog to 200,000) and
  a shadow-map fetch per fragment.
- **Bevy bloom with an HDR main view** and **Bevy CAS**: see §2.
- **Bevy volumetric fog** (`VolumetricFog` on the camera, `VolumetricLight` on the sun, a
  30,000 x 6,000 x 30,000-unit `FogVolume` following the camera, density 0.0005, 64 steps) at
  Goodsprings 19:00: the whole picture went dark brown and the volume's box edge showed as a
  hard step across the mountains [observed 2026-10-12, private frame
  `C:\fnv-private\scratch\graphics-2\vol-dusk.png`]. Its pass blends absorption and linear
  in-scattered light (times the exposure) onto the main texture, which in this non-HDR view
  already holds tonemapped display values, so it darkens without adding visible light; and a
  box big enough for the Mojave's distances needs coarse steps. It would also replace the
  weather's distance fog look rather than follow it. The sun shafts (§3.7) give the light
  shafts it was wanted for.

## 5. Frame rate and the defaults

### 5.1 Follow-ups (2026-10-12, this machine)

Measured on the machine that ran the follow-ups (GeForce RTX 4070, not the owner's GTX 1050),
1280 x 720, free camera standing still at Goodsprings 19:00 facing the sunset, with the test aids
`--log-frame-times 6 --graphics-cycle "vanilla;taa;light-shafts;enhanced;..."` (one run switching
between the sets every 6 s, three rounds; GPU time per pass from timestamp queries, Bevy's
`RenderDiagnosticsPlugin`, about the last second's average). The frame rate itself stayed at the
display's 60 Hz in every set (presented with vsync whatever was asked), so the GPU figures are
the measure:

| Pass | GPU ms |
|---|---|
| Main opaque pass, MSAA 4x / TAA (no multisampling) | 1.00-1.03 / 0.70-0.74 |
| Depth prepass, MSAA 4x / TAA | 0.32-0.33 / 0.22-0.23 |
| Temporal anti-aliasing pass | 0.04-0.05 |
| Sun shafts (four quarter-size passes) | 0.02-0.09, mostly 0.03 |
| Ambient occlusion (§3.1, for comparison) | 0.11-0.19 |
| Bloom and sharpening (§3.3-§3.4) | 0.03-0.05 |

So on this GPU each addition costs a few hundredths of a millisecond, and TAA saves about
0.4 ms against MSAA 4x. On the GTX 1050 (several times slower [hypothesis]) the same passes
would cost several times as much; not measured there.

### 5.2 First set (PR #236)

Measured 2026-10-12 on the owner's PC (GeForce GTX 1050 4 GB, 8 cores), 1280 × 720, Goodsprings
at noon (`--pos=-69000,3500,8900 --heading 30 --pitch -8`), free camera standing still,
`--run-seconds 60` (frames over the whole run, cell loading included), one after the other.
Other sessions' game runs and builds shared the machine, so the figures are noisy: the same
vanilla run gave 14.5 and later 7.2 frames per second.

| Run | Frames per second |
|---|---|
| `--graphics vanilla` (first) | 14.5 |
| `--graphics enhanced` (right after) | 10.0 |
| `--graphics-only fine-shadows` | 19.6 |
| `--graphics-only ambient-occlusion,bloom,sharpen,anisotropic` | 10.6 |
| `--graphics vanilla` (again, last) | 7.2 |

The back-to-back pair says enhanced costs about a third of the frame rate; the noise does not
allow a figure per feature. On this GPU the vanilla picture is already below 20 frames per
second, so:

- **Normal play defaults to `vanilla`** (`graphics::PLAY_DEFAULT`); `--graphics enhanced` or
  the settings page's "Graphics" switches turn the improvements on. On a faster GPU
  changing `PLAY_DEFAULT` to `Enhanced` is the one-line switch.
- `vanilla` is also the look of every test run: `--screenshot`, `--timelapse`, `--frame-json` and
  `--run-seconds` default to it whatever `PLAY_DEFAULT` says, so the parity lab, the picture
  checks and every smoke scenario keep their pictures, timings and expectations. `--graphics
  enhanced` on such a run asks for the enhanced look explicitly (the smoke scenario
  `graphics-enhanced` does).

### 5.3 Sun shadows (2026-10-12, playtest list 4 item 6)

The owner: "shadow rendering seriously affects performance; maybe limits on draw distance or
quality". Bevy times no shadow pass (they do not appear in the GPU list of `--log-frame-times`),
so the cost was measured as frame time with the cycle's shadow keys (§6) and the casters each
cascade drew (the log's "shadow casters per cascade" line). Machine: GeForce RTX 4070, 1280 × 800,
`--play` standing still at Goodsprings, (−69698, 2246), 14:00 looking east down the road (where
the owner's screenshots of item 5 were taken), `vanilla`, cycle switched every 4 s, three rounds
each; the figures are the round means (vsync off). The GPU's own time stayed between 1.9 and
4.1 ms in every set: the cost is the CPU's, one pass per cascade per view, each culling,
specialising and queueing its casters.

| Sun shadows | Casters per cascade | Frame time (ms) |
|---|---|---|
| off | | 7.3-7.9 |
| 8,000 in 2 cascades | 61, 2,000 | 12.4-13.8 |
| 8,000 in 3 cascades | 61, 340, 1,900 | 13.7-15.2 |
| 8,000 in 4 cascades | 61, 220, 710, 1,880 | 15.0-16.8 |
| 16,000 in 3 cascades | 200, 1,090, 3,060 | 16.4-18.3 |
| 16,000 in 4 cascades (the old default) | 200, 570, 1,690, 3,030 | 19.0-20.8 |

All with the arms' shade light (`player_body.rs`) taking the sun's first cascade, about 190
casters, as it did. Changed [engineering choices]:

- **The shade light reaches 256 units** from the camera (`ARMS_SHADE_REACH`): about 30 casters, and
  the old default 16,000 in 4 went from 19.0-20.8 to 16.9-17.6 ms. A cascade keeps every caster
  between it and the sun, so walls and roofs still shade the arms; their shadows on the arms
  are finer than before.
- **The sun's shadows default to 8,000 units in 3 cascades** (`sky::DEFAULT_SHADOW_DISTANCE`,
  `DEFAULT_SHADOW_CASCADES`; `--shadow-distance` still sets the reach): 12.5-12.9 ms against
  the old 19.0-20.8, about 7 ms (a third of the frame) saved; shadows off is 7.3-7.5. The first
  cascade, a twelfth of the reach, is now 667 units instead of 1,333, so shadows near the player
  are finer. Visible change: nothing farther than 8,000 units (about 114 m) casts a sun shadow;
  at noon the view down the road is unchanged
  (`C:\fnv-private\references\shadows_sun\shadow_limits_before_after.png`); at dusk the long
  shadows of distant rocks and buildings end nearer. The original casts no sun shadows from
  the world at all (world.md §6), so this is no further from its look.
- Finer shadows (§3.2) keep their 4096² maps and reach 1.5 × the distance, now 12,000
  (the 4096² maps went in #1367; the longer reach stays).

Not changed: the water's reflection camera, while active, gets its own cascades of the sun (Bevy
builds them for every active camera, with no way to leave one out), doubling the shadow passes
while water is in view; casters are not thinned by size with distance (no per-cascade filter in
Bevy; built and measured in §5.5).

### 5.4 Anti-aliasing and shadow-filter choices (2026-10-16, this machine)

GeForce RTX 4070 (not the owner's GTX 1050), 1280 × 720, free camera standing still at
Goodsprings 14:00 (`--pos=-69000,3500,8900 --heading 30 --pitch=-8 --sun-shadows`),
`--log-frame-times 4|5 --graphics-cycle` over vanilla (MSAA 4x), fxaa, smaa, taa and the two
shadow filters, five runs. Other sessions shared the machine; the frame time (11.8-12.5 ms in the
quiet runs) was the CPU's and did not tell the choices apart, so the GPU passes are the measure:

| Pass | GPU ms |
|---|---|
| Main opaque pass, MSAA 4x / without multisampling (FXAA, SMAA, TAA) | 0.72-0.78 / 0.56-0.58 |
| Depth prepass, MSAA 4x / without | 0.30-0.31 / 0.24 |
| FXAA pass | 0.04 |
| SMAA (its three passes) | 0.08-0.11 |
| Main opaque pass with the shadow filter Fast / Smooth / Temporal | 0.69-0.77 / 0.73-0.78 / 0.73-0.76 (within the noise) |

So FXAA and SMAA, like TAA, cost less than MSAA 4x in all (the main pass and the prepass save more
than their own pass), FXAA the least; the shadow filters' difference does not show at this
scale. On the GTX 1050 every figure would be several times larger [hypothesis, not measured
there], with the same order: MSAA 8x dearest, then 4x, then SMAA, TAA, FXAA.

One run stopped on a validation error ("A query of type PipelineStatistics was not ended") at
a switch into vanilla after the cycle had gone through SMAA and the shadow filters; four more
runs making the same switches (about 45 in all) did not repeat it. PipelineStatistics queries
come only from the measuring aid's `RenderDiagnosticsPlugin`; the smoke scenario
`graphics-settings`, which makes every switch without it, saw no error.

### 5.5 Where the sun shadows' cost goes (2026-10-23)

The owner: sun shadows drain performance, presumably the main thread. Measured with the span
profile (`NVF_PLAY_TEST_PROFILE`, a `--features nvforever/profile` build; frame-rate-audit.md
§1) and `--log-frame-times 8 --graphics-cycle`, so each comparison is one run. Desktop
(32 threads, RTX 4070), 1280 × 800, `--play` at the Goodsprings start, 14:00, standing still,
vanilla; other sessions built and ran the game at the same time, so only the cleanest rounds
(those that agree within about 0.5 ms) are quoted [observed].

**It is the render thread.** With shadows on the main thread's own work hardly changes
(about 7 against 6 ms a frame, Bevy's caster culling `check_dir_light_mesh_visibility` about
1 ms of it); the render thread goes from about 9.7 to 16.3 ms, and the main thread waits for
it (the `RenderExtractApp` span holds the wait). The render spans that grow: writing the
binned instance buffers (1.6 → 4.0 ms), preparing the preprocessing bind groups (1.2 → 2.4),
recording the passes (`RenderContextState::apply` 2.2 → 3.7) and submitting (`queue_submit`
1.3 → 2.4). The GPU's time stays small (§5.3).

| Sun shadows (8,000 units) | Casters per cascade | Frame (ms) | Render thread (ms) |
|---|---|---|---|
| off | | 10.2 | 9.7 |
| 1 cascade | 2,200 | 14.3 | 13.8 |
| 2 cascades | 85, 2,200 | 14.9 | 14.3 |
| 3 cascades (default) | 85, 460, 2,130 | 16.8 | 16.3 |

All with the arms' shade light (about 30 casters). So about 4 ms comes with having sun
shadows at all and the third cascade adds about 2 ms.

**Thinning the casters does not pay** (`shadow_thin.rs`, `--shadow-thin <ratio>`, off by
default): beyond 1,000 units a placed reference whose bounding radius is under `ratio` × its
distance casts nothing. The sun's last cascade, broken down by that angular size (the
frame-time log's `sun's last cascade:` line): about 75 meshes under 0.01, 450 at 0.01-0.02,
490 at 0.02-0.04, 340 at 0.04-0.08, 280 over 0.08, the rest near or not placed references.
Clutter whose shadow covers a few pixels is a small part.

| `--shadow-thin` | Meshes thinned | Last cascade | Render thread (ms) |
|---|---|---|---|
| 0 | 0 | 2,080-2,330 | 16.2-17.3 |
| 0.01 | 235 | 2,000-2,260 | 16.0-16.6 |
| 0.02 | 1,240 | 1,535-1,670 | 15.4-16.4 |
| 0.04 | 2,140 | 1,030-1,150 | 14.4-16.2 |

Halving the far cascade saves 1-2 ms of about 6.5, and at 0.04 an object under 2 m in radius
loses its shadow beyond 50 m: a visible change for little. The rule stays off.

**Why** [hypothesis, from Bevy 0.19.1's sources]: the per-view work scales with batch sets
more than with casters. `write_binned_instance_buffers` writes two GPU buffers and a GPU
unpacking job per batch set of every view, and the preprocessing bind groups are made per
view each frame. A shadow draw that only writes depth shares one batch set across materials,
but an alpha-tested NIF shape (plants, fences, signs) needs its material's textures, and
`NifMaterial` is not bindless, so each such material is a batch set of its own in every cascade.
Fewer casters rarely removes a whole material.

What would cut it, in order of what it saves:
- **A bindless `NifSurface`** (frame-rate-audit.md §4): alpha-tested materials would share
  batch sets in every view, the main view included. Built in §5.6: it takes the shadows'
  cost almost to nothing.
- **2 cascades instead of 3**: about 2 ms, with coarser shadows from 667 to about 2,300 units
  (the middle cascade's range; the first cascade stays 667). The owner's open choice from §5.3.
- **The arms' shade light** is one more shadow view while the first-person arms are drawn.
- **Water in view** gives the reflection camera a set of cascades of its own (§5.3).

### 5.6 Bindless NIF materials (2026-10-23)

`NifSurface`, the NIF material's extension of `StandardMaterial`, is bindless where the device
allows it, as `StandardMaterial` already was (Bevy turns bindless on for an extended material
only when both parts are): its values go to one storage array for all NIF materials and its
textures (holes, FaceGen offsets) to Bevy's shared texture arrays, and the shaders read them
through the index tables (`nif_surface_bindings.wesl`; without bindless the plain bindings
50-53). [engineering choice; nif_material.rs "Bindless"]

**Batch sets** (the profile aid's `profile batches:` line, Goodsprings start, 14:00, sun shadows
on) [observed]:

| | Before | Bindless |
|---|---|---|
| Shadow phases (4 views: 3 cascades and the arms' shade light) | ~830 | 49 |
| Main view, opaque / alpha-mask (and their prepasses) | 169 / 147 | 9 / 14 |

**Frame time** (same place, `--log-frame-times 8 --graphics-cycle "vanilla;vanilla,no-sun-shadows"`,
the span-profile build; the runs of each build alternated, and only uncapped windows are quoted:
in some runs the frames were held to the display's 60 Hz whatever the build, the window or the
compositor's doing) [observed]:

| | Before | Bindless |
|---|---|---|
| Sun shadows on (8,000 units, 3 cascades) | 16.5-17.3 ms | 6.4-6.6 ms |
| Sun shadows off | 9.9-10.0 ms | 6.7-6.8 ms |
| Render thread, shadows on / off | 16.3-17.0 / 10.0-10.4 ms | 6.3-8.3 / 5.2-6.1 ms |

The render spans that had grown with the shadows shrank most: writing the binned instance
buffers 4.0 → 0.4 ms, preparing the preprocessing bind groups 2.4 → 0.5, recording the passes
3.7 → 1.7, submitting 2.4 → 0.8. On the GPU the bin-unpacking pass went from about 1.7 to 0.15 ms.
The main thread is now about as busy as the render thread (6-8 ms), so the frame is no longer
held by either at this place; sun shadows cost next to nothing here.

**Same picture** [observed, screenshots of both builds at 1280 × 720 compared pixel by pixel]:
an NPC's face (Doc Mitchell, FaceGen tint: identical), the Pip-Boy (largest difference 12 of
255), the Prospector Saloon, Goodsprings at noon and dusk with sun shadows, a weapon in first
person, Gomorrah's roof fires: the only differences are what changes from run to run anyway
(wind-blown plants, a debug text line, HUD messages shown a moment apart). The refraction smoke
(`stealth-boy-refraction`, a moving bend) passes with both builds and its frames agree within
10 of 255. A build with the bindless attribute taken out (the fallback for devices without
bindless) drew the same face, Pip-Boy and first-person frames and passed the refraction smoke.
No pipeline or shader error was logged by any of the three builds.

Not measured: the owner's GTX 1050 (Bevy decides bindless per device; without it the plain
bindings are used, as before this change), and other places than Goodsprings.

### 5.7 Casters over the slice, not the square (2026-11-03, #1378)

The sun's shadows cost more at a low sun: at Novac the frame was 11.4 ms at 07:00 against 10.7
at noon, with the shadow passes' GPU time at 2.17 against 1.84 ms. Counting the casters each cascade
drew showed why [observed: a temporary probe of `CascadesVisibleEntities`]:

- Bevy fits each cascade to the light-space box around its slice of the camera's view and
  draws every caster in that box's square, plus everything between the box and the sun
  (`check_dir_light_mesh_visibility` skips the near plane). At 07:00 the sun stands 15°
  above the horizon, so more ground lies up-sun of each slice. That was 32, 201 and 13
  casters in the three cascades that lay wholly toward the sun past the near plane, against
  none at noon.
- Those are not waste. A test of whether any ray from the lowest ground could pass through
  them, given their height, removed 10 of them. They are the long morning shadows.
- The waste is the square. Seen from the sun, a slice of a perspective view is a trapezoid,
  and the square around it holds about as many casters again that shade nothing in the slice.
  This holds at every hour.

Changed (`sky.rs` `sun_casters_over_slice`, after Bevy's light visibility): each cascade
keeps only the casters whose light-space box overlaps the outline of its slice seen from
the sun, plus 8 shadow-map texels for the filter's reach. A cascade's receivers all lie in its
slice (the shader picks a cascade by view depth, and the overlap with the next cascade is in
both slices), and a caster can only shade what lies under it seen from the sun.

| Novac, casters per cascade | 07:00 | 12:00 |
|---|---|---|
| before | 154, 1,438, 3,248 | 120, 1,283, 2,928 |
| over the slice | 80, 761, 1,767 | 65, 480, 1,549 |

[observed 2026-11-03] Release builds, i9-14900K, RTX 4070, 1920 × 1080, `--graphics enhanced`,
`--play` standing still, `NVFOREVER_HOME` without stored settings. Each run is 45 s and logs its
frame time every 10 s; main and this change were interleaved over 3 rounds. One main run at 07:00
hit a throttled window at 16.67 ms and is left out.

| Novac | frame, main | frame, over the slice | shadow passes GPU, main / over the slice |
|---|---|---|---|
| 12:00 | 10.70-10.76 ms | 9.41-9.49 ms | 1.84 / 1.24 ms |
| 07:00 | 11.39-11.47 ms | 9.75-9.80 ms | 2.17 / 1.39 ms |

So 07:00 is now 0.33 ms from noon, against about 0.7 before. Screenshots of both builds at
07:00 and noon, from the free camera in four headings, differ only in wind-blown grass. The
casters dropped from the passes are still extracted for the frame (they stay visible to the
light), so the saving is in the passes, not in extraction.

## 6. Test aids for measuring

`--log-frame-times <seconds>` logs the mean and slowest frame time and the GPU time of the
costliest render passes every so many seconds (and asks for presenting without vsync, which the
driver may still impose); `--graphics-cycle "<set>;<set>;..."` switches between sets of
improvements after each logged period (`vanilla`, `enhanced` or keys as for `--graphics-only`;
the first half second after a switch is left out), so one run compares them under the same load.
A set may also change the sun's shadows: `no-sun-shadows`, `shadow-distance=<units>`,
`cascades=<n>`, `shadow-thin=<ratio>` (§5.5) among its keys
(`vanilla;no-sun-shadows;shadow-distance=16000,cascades=4`). The
anti-aliasing and shadow filters have keys too: `fxaa`, `smaa`, `taa`, `shadow-filter-hardware`,
`shadow-filter-temporal` (a set without one of them has `--msaa` and the Gaussian filter); the
log then gives the sun's shadow reach and cascades with each frame time, the sun's heading and
elevation, and the casters each cascade of each light drew; with sun shadows on also what
`--shadow-thin` thinned and the sun's last cascade broken down by angular size (§5.5).

## 7. Next candidates

- Bloom and eye adaptation driven by the image spaces' own HDR values (IMGS DNAM's 14 HDR
  floats and 3 bloom floats), once their meanings are documented from xEdit: that would move
  bloom from an engine choice towards parity.
- Bevy SSAO or GTAO once the custom prepasses write normals; motion vectors from the custom
  prepasses (last frame's wind and skinning) would let the TAA follow what moves by itself.
- Sun shafts through interior windows are the game's own light-ray meshes already; none from
  the moon.
- A shadow distance and map size setting with more steps than on/off (the cycle keys of §6
  measure them).
