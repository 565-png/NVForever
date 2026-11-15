// Height-based blending of the folded landscape layers (terrain.wesl with
// TERRAIN_HEIGHT_BLEND; terrain.rs "Height blending"; docs/design/graphics-enhanced.md
// §3.12) [design, #2550]. Plain WGSL on purpose: no imports and no conditions, so the
// engine's tests parse and validate this very file with naga (terrain.rs tests), and
// terrain.rs `height_weights` mirrors it on the CPU.
//
// Seven entries: the base at index 0, folded layer k at k + 1. Every constant is
// NVForever's own starting choice, to be settled from pictures; none is a fact about the
// original game.

// The luminance weights (Rec. 709) applied to linear colour.
const TERRAIN_LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);
// Height = gain x (texel luminance - texture mean luminance), clamped to +- limit.
const TERRAIN_HEIGHT_GAIN: f32 = 1.0;
const TERRAIN_HEIGHT_LIMIT: f32 = 0.25;
// A height one band above the share-weighted mean doubles a layer's weight (before the
// weights are made to sum to 1 again); the exponent is clamped to +- 4 bands.
const TERRAIN_HEIGHT_BAND: f32 = 0.10;
// Heights whose share-weighted spread (standard deviation) is below this take today's
// linear shares; the height weighting is fully on at twice it.
const TERRAIN_EQUAL_EPS: f32 = 0.005;

// A texel's height: its luminance less its texture's mean luminance (mean-centring: a
// bright texture does not win for being bright, but the texture's broad shading stays;
// this is not a spatial high-pass filter). Both colours are linear (the textures are
// read sRGB-decoded).
fn terrain_height(texel: vec3<f32>, mean: vec3<f32>) -> f32 {
    return clamp(TERRAIN_HEIGHT_GAIN * (dot(texel, TERRAIN_LUMA) - dot(mean, TERRAIN_LUMA)), -TERRAIN_HEIGHT_LIMIT, TERRAIN_HEIGHT_LIMIT);
}

// The share of the base and of each of the `n` layers in the ordered blend: `a[k]` is
// layer k's painted share (opacity x the layer texture's alpha), each layer laid over
// what came before it, as the `mix` chain of terrain.wesl's ordinary path does.
// w[k + 1] = a[k] x the product of (1 - a[j]) over the later layers j; w[0] the base's
// remainder. They sum to 1.
fn terrain_ordered_shares(a: array<f32, 6>, n: u32) -> array<f32, 7> {
    var paint = a;
    var w = array<f32, 7>(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    var remaining = 1.0;
    for (var i = i32(min(n, 6u)) - 1; i >= 0; i -= 1) {
        let s = clamp(paint[i], 0.0, 1.0);
        w[i + 1] = s * remaining;
        remaining *= 1.0 - s;
    }
    w[0] = remaining;
    return w;
}

// The final weights from the ordered shares `w` and the heights `h`: each share boosted by
// 2^((height - mean height) / band), the exponent clamped to +- 4, then made to sum to 1
// again. Where the contributing heights are about equal the shares stay linear (a smooth
// gate on their spread), so flat textures do not draw hard triangle edges. A share of 0
// stays 0 and a sole contributor keeps 1, so what the data paints fully, or not at all,
// looks as without height blending.
fn terrain_height_weights(w: array<f32, 7>, h: array<f32, 7>) -> array<f32, 7> {
    var p = w;
    var height = h;
    var total = 0.0;
    for (var i = 0; i < 7; i += 1) {
        p[i] = max(p[i], 0.0);
        total += p[i];
    }
    if total <= 0.0 {
        return array<f32, 7>(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    }
    var mean = 0.0;
    for (var i = 0; i < 7; i += 1) {
        p[i] = p[i] / total;
        mean += p[i] * height[i];
    }
    var spread = 0.0;
    for (var i = 0; i < 7; i += 1) {
        let d = height[i] - mean;
        spread += p[i] * d * d;
    }
    let gate = smoothstep(TERRAIN_EQUAL_EPS * TERRAIN_EQUAL_EPS, 4.0 * TERRAIN_EQUAL_EPS * TERRAIN_EQUAL_EPS, spread);
    var q = array<f32, 7>(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    var q_total = 0.0;
    for (var i = 0; i < 7; i += 1) {
        let e = clamp((height[i] - mean) / TERRAIN_HEIGHT_BAND, -4.0, 4.0);
        q[i] = p[i] * mix(1.0, exp2(e), gate);
        q_total += q[i];
    }
    for (var i = 0; i < 7; i += 1) {
        q[i] = q[i] / q_total;
    }
    return q;
}
