// Sun-projected moving light patches on submerged opaque receivers [design, #2674].
// All positions and the direction toward the sun use Bevy world space (y up).
fn nvf_caustics_hash(p: vec2<f32>) -> f32 {
    let h = fract(vec3(p.x, p.y, p.x) * 0.1031);
    let mixed = h + dot(h, h.yzx + 33.33);
    return fract((mixed.x + mixed.y) * mixed.z);
}

fn nvf_caustics_noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let local = fract(p);
    // Quintic interpolation keeps the light and its slope smooth at cell edges.
    let blend = local * local * local * (local * (local * 6.0 - 15.0) + 10.0);
    return mix(
        mix(nvf_caustics_hash(cell), nvf_caustics_hash(cell + vec2(1.0, 0.0)), blend.x),
        mix(nvf_caustics_hash(cell + vec2(0.0, 1.0)), nvf_caustics_hash(cell + vec2(1.0)), blend.x),
        blend.y,
    );
}

// Returns fractional light gain, to multiply the underlying colour by 1 + gain.
fn nvf_caustics(receiver: vec3<f32>, surface_y: f32, sun_dir: vec3<f32>, daylight: f32, time: f32) -> f32 {
    let depth = surface_y - receiver.y;
    if daylight <= 0.0 || depth <= 0.0 || depth >= 600.0 || sun_dir.y <= 0.0 {
        return 0.0;
    }
    let sun = sun_dir / max(length(sun_dir), 0.001);
    // Trace toward the surface using Snell's law (air/water index ratio 0.75).
    // The refracted vertical component stays safely away from zero at the horizon.
    let horizontal = sun.xz * 0.75;
    let vertical = sqrt(max(1.0 - dot(horizontal, horizontal), 0.001));
    let at_surface = receiver.xz + horizontal * (depth / vertical);
    // Broad, softly blended irregular patches suit a dusty lake bed. Separate
    // drifting scales vary the shape without closed ridges or a repeating tile.
    let p = at_surface / 65.0;
    let broad = nvf_caustics_noise(p + vec2(time * 0.031, -time * 0.023));
    let detail_p = vec2(p.x * 1.67 + p.y * 0.59, p.y * 1.67 - p.x * 0.59);
    let detail = nvf_caustics_noise(detail_p + vec2(17.3 - time * 0.019, 8.7 + time * 0.027));
    let illumination = smoothstep(0.34, 0.79, broad * 0.72 + detail * 0.28);
    let envelope = smoothstep(0.0, 2.0, depth) * (1.0 - smoothstep(80.0, 600.0, depth));
    return 0.28 * illumination * envelope * clamp(daylight, 0.0, 1.0)
        * smoothstep(0.0, 0.18, sun.y);
}
