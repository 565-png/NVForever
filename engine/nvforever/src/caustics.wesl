// Sun-projected moving light ridges on submerged opaque receivers [design, #2674].
// All positions and the direction toward the sun use Bevy world space (y up).
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
    // A warped triangular lattice gives connected pools of light. The two
    // staggered rows below yield the nearest centre without a neighbour search.
    let p = at_surface / 30.0;
    let drift = vec2(time * 0.085, -time * 0.065);
    let long_wave = vec2(sin(p.y * 1.4 + time * 0.51), sin(p.x * 1.6 - time * 0.56));
    let short_wave = vec2(sin(p.x * 2.8 + p.y + time * 0.61), sin(p.y * 2.2 - p.x - time * 0.73));
    let q = p + drift + long_wave * 0.29 + short_wave * 0.12;
    let spacing = vec2(1.0, 1.73205);
    let a = (fract(q / spacing + 0.5) - 0.5) * spacing;
    let b = (fract(q / spacing) - 0.5) * spacing;
    let local = abs(select(a, b, dot(b, b) < dot(a, a)));
    let edge = 0.5 - max(local.x, dot(local, vec2(0.5, 0.866025)));
    let ridge = 1.0 - smoothstep(0.0, 0.105, edge);
    let envelope = smoothstep(0.0, 2.0, depth) * (1.0 - smoothstep(80.0, 600.0, depth));
    return 0.95 * ridge * ridge * envelope * clamp(daylight, 0.0, 1.0)
        * smoothstep(0.0, 0.18, sun.y);
}
