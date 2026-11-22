# HDR output

Optional, off by default. With **HDR output** on (Display page; `NVF_HDR=1` for a run),
the game draws to an HDR screen through Windows' scRGB swapchain. Below paper white the
picture is the SDR picture, exactly, unless the player asks for highlight expansion. Light
that the SDR picture cuts off at white (the sun, fire, explosions, neon, lamps) is shown
brighter than paper white, up to the screen's peak, and never cut.

## Requirements

- Windows HDR on for the screen (Settings, Display, Use HDR).
- A Vulkan surface that offers `Rgba16Float` with the extended linear sRGB colour space
  [observed 2026-11-21: an NVIDIA laptop offers it only in its discrete GPU mode; in hybrid
  mode the surface offers 8-bit sRGB only]. Without it the game logs
  `hdr: this surface offers no Rgba16Float; staying SDR` and draws the SDR picture.
- The setting takes effect at the next start (the surface format is chosen when the window's
  surface is made). Every calibration row works live.

## Signal path

scRGB is linear light with Rec.709 primaries, 1.0 = 80 nits [documented: Microsoft,
"High dynamic range and wide color gamut", scRGB; wgpu `SurfaceColorSpace::Auto` picks it
for an `Rgba16Float` surface]. The game's own values are display values, 1.0 the brightest
an SDR screen shows (world.md §8). The path, in order:

1. **Tonemapping pass** (lights.rs `GAME_HDR`): the SDR build holds each channel to 1 (the
   original's clip, `GAME_CLIP`). With an fp16 surface the clip is left out, so light above
   1 reaches the grade. The choice waits until the window's surface is made
   (`NVF_HDR_SURFACE_DECIDED`): on an SDR surface the clip stays.
2. **Hot light** (nif_material.wesl `hot_light`, shader def `NVF_HDR`): below white the
   SDR result; above white the light the shape emits, with its emissive multiplier (a
   lantern's glass x 10, a neon tube x 25), so a neon tube ends brighter than a lantern.
   Held to 1 per channel the two are the same.
3. **Image-space grade** (imagespace.wesl): the grade works on the picture held to 1, as in
   SDR. The light above 1 is graded apart (saturation, tint, brightness, VATS, fade) and
   added back. With SDR output this part is zero.
4. **HDR pass** (hdr_expand.wesl, after post-processing, before the HUD):
   1. Highlight expansion: above the knee (0.6, brightest channel) the SDR picture is
      stretched so its white lands at paper white plus the expansion's share of 60 % of
      the room to the peak. The stretch grows with the square of the distance from the
      knee, so the slope at the knee is unchanged. Colour ratios are kept. At 0 % the
      picture is the SDR picture.
   2. Light sources: the light above SDR white, times the source gain, rises from there
      towards the peak on a Reinhard shoulder of its brightest channel (slope 1 at the
      start); the other channels are scaled by the same factor, so the light's colour
      stays. No value passes the peak, none is cut.
   3. Hue restore: a changed pixel's hue is turned towards the SDR picture's hue in Oklab
      [documented: Björn Ottosson, "A perceptual color space for image processing", 2020],
      keeping its lightness and chroma. At 100 % bright light keeps the colour the
      original's per-channel clip gives it (a lantern's wall burns yellow-white, as in the
      original's frames). The same idea as RenoDX's hue correction [documented:
      clshortfuse/renodx wiki, "Hue Correction"].
5. **Window blit** (vendored bevy_core_pipeline `blit.wesl`): the 0..1 range is decoded
   with gamma 2.2 (setting **HDR gamma 2.2**, on by default), then everything is scaled so
   the HUD's white is the HUD brightness in nits. The HUD is drawn in the same pass at the
   same scale.

Gamma 2.2: an SDR screen decodes with a 2.2 power curve; decoding the game's sRGB-encoded
values with the piecewise sRGB curve instead lifts the shadows [documented: the sRGB curve's
linear segment below 0.04045]. Values above 1 are left as they are.

## Settings (Display page)

| Row | Range | Default | What it sets |
| --- | --- | --- | --- |
| HDR output | on, off | off | the swapchain, next start |
| HDR paper white | 80 to 400 nits | 200 | the picture's white |
| HDR peak brightness | Auto, 400 to 2000 nits | Auto | the brightest value written |
| HDR HUD brightness | 80 to 400 nits | 200 | the HUD's and menus' white |
| HDR gamma 2.2 | on, off | on | the decode of the 0..1 range |
| HDR highlight colour | 0 to 100 % | 100 % | the hue restore |
| HDR highlight expansion | 0 to 100 % | 50 % | step 4.1 |
| HDR light source brightness | x1 to x16 | x6 | the source gain, step 4.2 |
| HDR brightness heatmap | on, off | off | the picture replaced by the heatmap |

Every default and range is a design choice [design]. Paper white 200 nits is near ITU-R
BT.2408's reference white (203 nits) [documented: ITU-R BT.2408].

**Auto peak** reads the screen's peak from Windows: `IDXGIOutput6::GetDesc1`,
`MaxLuminance` of the output in HDR mode (PQ, BT.2020) [documented: DXGI_OUTPUT_DESC1]. The
figure comes from the screen's EDID, or from the Windows HDR Calibration app's profile once
it has run. It follows the screen the window is on: winit names a monitor by the same
device name DXGI reports (`\\.\DISPLAY1`) [hypothesis: both come from `GetMonitorInfo`'s
`szDevice`]. Without a match, the brightest HDR output's figure; without any, 600 nits
[design]. Environment overrides for a run: `NVF_HDR_PAPER_WHITE`, `NVF_HDR_PEAK`,
`NVF_HDR_HEATMAP=1`.

## Heatmap

**HDR brightness heatmap** shows each pixel's output: grey (its SDR luminance) up to paper
white, then by the share of the room to the peak cyan (to 25 %), green (50 %), yellow (75 %),
orange, and red from 98 %. It is meant for calibration and for checking that nothing is
cut: a red area is at the peak, not past it.

[observed 2026-11-22, Goodsprings at 13:00 and 19:00, expansion 0, laptop screen 616 nits]:
no pixel was above paper white. The game's light is authored in display values, so in a
plain daylight view only the sun, fire, emitters and stacked effects pass white. With the
expansion at 50 % the brightest clouds rose 25 to 50 % into the room.

## Not done

- Wide gamut: the output stays within Rec.709 primaries (the game's content is sRGB).
- An HDR10 (PQ) swapchain: scRGB only; Windows converts for the screen.
- Exclusive full screen: not tested with HDR.
