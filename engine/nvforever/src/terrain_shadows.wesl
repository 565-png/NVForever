// Terrain shadow heights (#2671, stage T1; terrain_shadows.rs): for every texel of the sun
// heightfield (sun_heightfield.rs, R height, G 1 valid / 0 invalid, row 0 the southernmost),
// the height S under which a receiver over that texel is in the landscape's shadow:
//   S = max over the texels the march visits of [H(texel) - entry(texel) * tan(elevation)],
// entry the horizontal distance at which the ray towards the light enters that texel. Output
// R = S, G = the horizontal distance (world units) to the texel that gave S, both
// NO_SHADOW / 0 where nothing casts. Plain WGSL (no imports, no shader defs), so the module
// test validates exactly this text. terrain_shadows.rs `march_texel` mirrors `march` line for
// line; change both together.

struct Params {
    // Unit horizontal direction towards the light in texel space: x east (+column), y north (+row).
    dir: vec2<f32>,
    // Height lost per texel of horizontal distance: tan(elevation) * texel.
    tan_texel: f32,
    // World units per texel (for G).
    texel: f32,
    // Highest and lowest valid height of the map.
    top: f32,
    bottom: f32,
    // Texels per side.
    side: u32,
    // The most samples per texel (N).
    steps: u32,
    // The first steps' length in texels.
    step_min: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

const NO_SHADOW: f32 = -1.0e30;

@group(0) @binding(0) var heights: texture_2d<f32>;
@group(0) @binding(1) var shadow_out: texture_storage_2d<rg32float, write>;
@group(0) @binding(2) var<uniform> params: Params;

// Where the ray from p along d enters the span [c, c + 1] on one axis (texel units); an axis
// the ray does not move along never limits the entry.
fn axis_entry(c: f32, p: f32, d: f32) -> f32 {
    if abs(d) < 1.0e-6 {
        return -1.0e30;
    }
    let edge = select(c + 1.0, c, d > 0.0);
    return (edge - p) / d;
}

// Horizontal distance (texels) from p along d to the map's edge.
fn edge_distance(p: vec2<f32>, d: vec2<f32>, side: f32) -> f32 {
    var t = 1.0e30;
    if d.x > 1.0e-6 {
        t = min(t, (side - p.x) / d.x);
    } else if d.x < -1.0e-6 {
        t = min(t, -p.x / d.x);
    }
    if d.y > 1.0e-6 {
        t = min(t, (side - p.y) / d.y);
    } else if d.y < -1.0e-6 {
        t = min(t, -p.y / d.y);
    }
    return t;
}

// The march from texel `own`'s centre (terrain_shadows.rs module notes, "The march").
fn march(own: vec2<i32>) -> vec2<f32> {
    let side = i32(params.side);
    let p = vec2<f32>(own) + vec2<f32>(0.5, 0.5);
    let d = params.dir;
    // Past this distance nothing can rise above the lowest valid height, and past the
    // map's edge nothing is known.
    let reach = min(edge_distance(p, d, f32(params.side)), max(params.top - params.bottom, 0.0) / params.tan_texel);
    let n = min(params.steps, u32(ceil(max(reach, 0.0) / params.step_min)));
    let b = max(reach - f32(params.steps) * params.step_min, 0.0) / (f32(params.steps) * f32(params.steps));
    var best = NO_SHADOW;
    var best_t = 0.0;
    var last = own;
    for (var i = 1u; i <= n; i = i + 1u) {
        let fi = f32(i);
        let t = params.step_min * fi + b * fi * fi;
        let c = vec2<i32>(floor(p + t * d));
        if c.x < 0 || c.y < 0 || c.x >= side || c.y >= side {
            break;
        }
        // The receiver's own texel (its maximum is the receiver's own ground) and a texel
        // already visited are left out.
        if all(c == last) {
            continue;
        }
        last = c;
        let entry = max(max(axis_entry(f32(c.x), p.x, d.x), axis_entry(f32(c.y), p.y, d.y)), 0.0);
        let fall = entry * params.tan_texel;
        // Every texel after this one is entered later: none can beat `best` any more.
        if params.top - fall <= best {
            break;
        }
        let h = textureLoad(heights, c, 0);
        if h.g > 0.5 {
            let s = h.r - fall;
            if s > best {
                best = s;
                best_t = entry;
            }
        }
    }
    if best <= NO_SHADOW {
        return vec2<f32>(NO_SHADOW, 0.0);
    }
    return vec2<f32>(best, best_t * params.texel);
}

@compute @workgroup_size(8, 8, 1)
fn produce(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.side || id.y >= params.side {
        return;
    }
    let own = vec2<i32>(id.xy);
    var result = vec2<f32>(NO_SHADOW, 0.0);
    // An invalid receiver texel gets no shadow; so does every texel when there is no light
    // to march towards (steps 0).
    if textureLoad(heights, own, 0).g > 0.5 && params.steps > 0u {
        result = march(own);
    }
    textureStore(shadow_out, own, vec4<f32>(result, 0.0, 1.0));
}
