// Landscape anti-tiling: where the ground textures are read so their repeat does not show
// as a grid (nvf-assets terrain_sampling.rs; nvforever terrain.rs "Anti-tiling";
// docs/design/graphics-enhanced.md §3.13) [design, #2551]. Plain WGSL on purpose: no
// imports and no conditions, so the tests parse and validate this very file with naga and
// mirror it on the CPU (terrain_sampling.rs tests). terrain_pbr.wesl puts it to work.
//
// The texture coordinate (game x, y over the texture repeat, as the landscape meshes carry
// it) is laid on a lattice of equilateral triangles four repeats on a side. Each lattice
// vertex (a "corner", named by two signed whole numbers) gives the texture its own offset
// (and, for Hex3, one of six rotations), hashed from its name, so the same place always
// reads the same way, across quadrant and cell edges too. A fragment reads the texture once
// for each corner of its triangle that it keeps (Triangle2: the two nearest; Hex3: all
// three) and blends the reads by weights from its place in the triangle. The techniques
// are published (Heitz & Neyret, HPG 2018, "High-Performance By-Example Noise using a
// Histogram-Preserving Blending Operator"; Mikkelsen, JCGT 11(3) 2022, "Practical
// Real-Time Hex-Tiling"; Quilez, "Texture repetition"); this is NVForever's own small
// version of them (no histogram transform), and the two-strongest-corners variant is
// known from Skyrim Community Shaders' "Terrain Variation" (prior art only; no code taken
// from any of them). Every constant is NVForever's own starting choice, to be settled from
// pictures; none is a fact about the original game, which repeats each texture plainly.

// sqrt(3) / 2: the height of an equilateral triangle of side 1.
const TERRAIN_HEX_H: f32 = 0.8660254037844386;
// The lattice's side, in texture repeats (1024 world units at the game's 256).
const TERRAIN_LATTICE_REPEATS: f32 = 4.0;
// Hash salts of a corner's offset (x, y) and rotation.
const TERRAIN_SALT_OFFSET_X: u32 = 0x25510001u;
const TERRAIN_SALT_OFFSET_Y: u32 = 0x25510002u;
const TERRAIN_SALT_ROTATION: u32 = 0x25510003u;
// A tap's weight below this fades to 0 and its read is skipped (1 / 256).
const TERRAIN_FAINT_TAP: f32 = 0.00390625;

// The fragment's texture coordinate and its change across one view pixel (already scaled
// by the view's mip bias: a gradient read takes no bias).
struct TerrainFootprint {
    uv: vec2<f32>,
    dx: vec2<f32>,
    dy: vec2<f32>,
}

// One read: where (texture coordinate), the rotation from the surface's texture frame to
// the read's (identity for Triangle2) and its weight (0: not read).
struct TerrainTap {
    uv: vec2<f32>,
    rotation: mat2x2<f32>,
    weight: f32,
}

// The reads of one fragment, the same for every texture it reads; `count` of them are used.
struct TerrainTaps {
    taps: array<TerrainTap, 3>,
    count: u32,
}

// The fragment's lattice triangle: its corners' names, the fragment's barycentric
// weights and its texture coordinate relative to each corner (unrotated).
struct TerrainLattice {
    id: array<vec2<i32>, 3>,
    bary: vec3<f32>,
    rel: array<vec2<f32>, 3>,
}

// A 32-bit integer hash (the "lowbias32" mixer); whole-number arithmetic wraps.
fn terrain_mix32(x: u32) -> u32 {
    var h = x;
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    return h ^ (h >> 16u);
}

// A corner's hash for one salt. Negative names are taken bit for bit (two's complement).
// The salt is mixed into the inner hash, not laid over the outer one: XORed on the outside,
// salts that differ in their last bits gave one corner's offset the hash of its neighbour's
// rotation (x ^ salt is the same for x = 0, salt 2 and x = 1, salt 3).
fn terrain_corner_hash(id: vec2<i32>, salt: u32) -> u32 {
    return terrain_mix32(bitcast<u32>(id.x) ^ terrain_mix32(bitcast<u32>(id.y) ^ 0x9e3779b9u ^ terrain_mix32(salt)));
}

// The hash's top 24 bits as a number in 0..1.
fn terrain_unit_float(h: u32) -> f32 {
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

// The lattice triangle holding texture coordinate `uv`. The lattice's basis is
// (1, 0) and (1/2, H) times its side; `q` is `uv` in that basis.
fn terrain_lattice(uv: vec2<f32>) -> TerrainLattice {
    let p = uv / TERRAIN_LATTICE_REPEATS;
    let q = vec2<f32>(p.x - p.y / (2.0 * TERRAIN_HEX_H), p.y / TERRAIN_HEX_H);
    let fl = floor(q);
    let cell = vec2<i32>(fl);
    let f = q - fl;
    var d = array<vec2<i32>, 3>(vec2<i32>(0, 0), vec2<i32>(1, 0), vec2<i32>(0, 1));
    var b = vec3<f32>(1.0 - f.x - f.y, f.x, f.y);
    if f.x + f.y > 1.0 {
        d = array<vec2<i32>, 3>(vec2<i32>(1, 1), vec2<i32>(0, 1), vec2<i32>(1, 0));
        b = vec3<f32>(f.x + f.y - 1.0, 1.0 - f.x, 1.0 - f.y);
    }
    // Rounding can leave a weight a hair below 0.
    b = max(b, vec3<f32>(0.0));
    b = b / (b.x + b.y + b.z);
    var out: TerrainLattice;
    out.bary = b;
    for (var j = 0; j < 3; j += 1) {
        out.id[j] = cell + d[j];
        let e = f - vec2<f32>(d[j]);
        out.rel[j] = TERRAIN_LATTICE_REPEATS * vec2<f32>(e.x + 0.5 * e.y, TERRAIN_HEX_H * e.y);
    }
    return out;
}

// A corner's offset of the texture (0..1 in each direction).
fn terrain_corner_offset(id: vec2<i32>) -> vec2<f32> {
    return vec2<f32>(terrain_unit_float(terrain_corner_hash(id, TERRAIN_SALT_OFFSET_X)), terrain_unit_float(terrain_corner_hash(id, TERRAIN_SALT_OFFSET_Y)));
}

// Rotation `r` (0..5) of six, by 60 degrees each: columns (c, s) and (-s, c).
fn terrain_rotation(r: u32) -> mat2x2<f32> {
    var c = array<f32, 6>(1.0, 0.5, -0.5, -1.0, -0.5, 0.5);
    var s = array<f32, 6>(0.0, TERRAIN_HEX_H, TERRAIN_HEX_H, 0.0, -TERRAIN_HEX_H, -TERRAIN_HEX_H);
    let i = min(r, 5u);
    return mat2x2<f32>(vec2<f32>(c[i], s[i]), vec2<f32>(-s[i], c[i]));
}

// The faint-tap rule: each weight less TERRAIN_FAINT_TAP (held at 0), made to sum to 1
// again, so a tap fades to 0 before its read is skipped (no jump at the cut). The largest
// of two or three weights summing to 1 is at least 1/3, so one always survives.
fn terrain_drop_faint(taps: TerrainTaps) -> TerrainTaps {
    var out = taps;
    var total = 0.0;
    for (var j = 0u; j < 3u; j += 1u) {
        let v = select(0.0, max(out.taps[j].weight - TERRAIN_FAINT_TAP, 0.0), j < out.count);
        out.taps[j].weight = v;
        total += v;
    }
    for (var j = 0u; j < 3u; j += 1u) {
        out.taps[j].weight = out.taps[j].weight / total;
    }
    return out;
}

// Triangle2: the two corners with the largest weights (equal weights: the smaller name,
// x first, then y), offsets only. Where the kept pair changes, a read at a nonzero weight
// is swapped for another: not continuous (the comparison decides whether it shows).
fn terrain_triangle2(uv: vec2<f32>) -> TerrainTaps {
    let l = terrain_lattice(uv);
    var id = l.id;
    var rel = l.rel;
    var b = array<f32, 3>(l.bary.x, l.bary.y, l.bary.z);
    // The corner left out: the weakest, ties going to the larger name.
    var drop = 0;
    for (var j = 1; j < 3; j += 1) {
        let weaker = b[j] < b[drop];
        let tie_larger = b[j] == b[drop] && (id[j].x > id[drop].x || (id[j].x == id[drop].x && id[j].y > id[drop].y));
        if weaker || tie_larger {
            drop = j;
        }
    }
    var out: TerrainTaps;
    out.count = 2u;
    var n = 0u;
    var total = 0.0;
    for (var j = 0; j < 3; j += 1) {
        if j != drop {
            total += b[j];
        }
    }
    let identity = mat2x2<f32>(vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0));
    for (var j = 0; j < 3; j += 1) {
        if j != drop {
            out.taps[n] = TerrainTap(rel[j] + terrain_corner_offset(id[j]), identity, b[j] / total);
            n += 1u;
        }
    }
    out.taps[2] = TerrainTap(vec2<f32>(0.0), identity, 0.0);
    return terrain_drop_faint(out);
}

// Hex3: all three corners, each a hex cell's centre with its own offset and rotation; the
// weights are the barycentric weights to the fourth power, made to sum to 1 (a narrower
// blend than linear; not the cited histogram transform). Continuous: a corner's weight is
// 0 on the edge across from it.
fn terrain_hex3(uv: vec2<f32>) -> TerrainTaps {
    let l = terrain_lattice(uv);
    var id = l.id;
    var rel = l.rel;
    let b2 = l.bary * l.bary;
    let b4 = b2 * b2;
    let w = b4 / (b4.x + b4.y + b4.z);
    var out: TerrainTaps;
    out.count = 3u;
    for (var j = 0; j < 3; j += 1) {
        let r = terrain_rotation(terrain_corner_hash(id[j], TERRAIN_SALT_ROTATION) % 6u);
        out.taps[j] = TerrainTap(r * rel[j] + terrain_corner_offset(id[j]), r, w[j]);
    }
    return terrain_drop_faint(out);
}

// A texture read at every kept tap with explicit gradients (the unoffset coordinate's,
// turned with the tap), blended by the taps' weights. Gradient reads are allowed where
// the fragments of a quad take different branches. For textures bound one by one; a
// bindless table's texture is read in the caller (terrain_pbr.wesl), so its index keeps
// its non-uniform marking.
fn terrain_sample_diffuse(tex: texture_2d<f32>, samp: sampler, taps: TerrainTaps, fp: TerrainFootprint) -> vec4<f32> {
    var t = taps;
    var sum = vec4<f32>(0.0);
    for (var j = 0u; j < t.count; j += 1u) {
        let tap = t.taps[j];
        if tap.weight > 0.0 {
            sum += tap.weight * textureSampleGrad(tex, samp, tap.uv, tap.rotation * fp.dx, tap.rotation * fp.dy);
        }
    }
    return sum;
}

// One normal-map texel decoded to a tangent-space normal in the surface's texture frame:
// x, y from 0..1 to -1..1, z the same or (two-component maps) rebuilt from x and y, green
// flipped where the map's points down the image, then turned back by the tap's rotation
// (its inverse, the transpose), after the decoding and the flip.
fn terrain_normal_tap(encoded: vec3<f32>, two_component: bool, flip_y: bool, rotation: mat2x2<f32>) -> vec3<f32> {
    var n = vec3<f32>(encoded.xy * 2.0 - 1.0, encoded.z * 2.0 - 1.0);
    if two_component {
        n.z = sqrt(max(0.0, 1.0 - dot(n.xy, n.xy)));
    }
    if flip_y {
        n.y = -n.y;
    }
    return vec3<f32>(transpose(rotation) * n.xy, n.z);
}

// The blended tangent-space normal made unit length; flat where the blend cancels out.
fn terrain_unit_normal(sum: vec3<f32>) -> vec3<f32> {
    return select(vec3<f32>(0.0, 0.0, 1.0), normalize(sum), dot(sum, sum) > 1e-12);
}

// A normal map read at every kept tap (as terrain_sample_diffuse), each decoded and turned
// back (terrain_normal_tap), blended and made unit length.
fn terrain_sample_normal(tex: texture_2d<f32>, samp: sampler, taps: TerrainTaps, fp: TerrainFootprint, two_component: bool, flip_y: bool) -> vec3<f32> {
    var t = taps;
    var sum = vec3<f32>(0.0);
    for (var j = 0u; j < t.count; j += 1u) {
        let tap = t.taps[j];
        if tap.weight > 0.0 {
            let encoded = textureSampleGrad(tex, samp, tap.uv, tap.rotation * fp.dx, tap.rotation * fp.dy).rgb;
            sum += tap.weight * terrain_normal_tap(encoded, two_component, flip_y, tap.rotation);
        }
    }
    return terrain_unit_normal(sum);
}

// A tangent-space normal `n` in the surface's frame `tbn` (columns T, B, N, as Bevy's
// calculate_tbn_mikktspace gives them), turned round first where `flip` (the back of a
// double-sided surface), as Bevy's apply_normal_mapping does after its decoding.
fn terrain_world_normal(tbn: mat3x3<f32>, n: vec3<f32>, flip: bool) -> vec3<f32> {
    let t = select(n, -n, flip);
    return normalize(t.x * tbn[0] + t.y * tbn[1] + t.z * tbn[2]);
}

// The texture coordinate of a point (Bevy world space; game (x, y) is Bevy (x, -z)) for
// textures repeating every `repeat` world units: u = x / repeat, v = y / repeat, as the
// landscape meshes carry it (nvforever terrain.rs `quadrant_mesh`).
fn terrain_world_uv(world: vec3<f32>, repeat: f32) -> vec2<f32> {
    return vec2<f32>(world.x, -world.z) / repeat;
}
