use super::base::NoiseContext;
use super::composite::fbm;

/// Marble: sine wave warped by noise, producing veined patterns.
pub fn marble(ctx: &NoiseContext, x: f64, y: f64, octaves: u32, warp_strength: f64) -> f64 {
    let warp = fbm(ctx, x, y, octaves, 2.0, 0.5) * warp_strength;
    ((x + warp) * std::f64::consts::PI).sin()
}

/// Domain warp: multi-pass noise distortion for organic patterns.
pub fn domain_warp(ctx: &NoiseContext, x: f64, y: f64, octaves: u32, warp_strength: f64) -> f64 {
    let (wx, wy) = first_warp_pass(ctx, x, y, octaves, warp_strength);
    let (wx2, wy2) = second_warp_pass(ctx, x, y, wx, wy, octaves, warp_strength);
    fbm(ctx, x + wx2, y + wy2, octaves, 2.0, 0.5)
}

/// Stripe: directional lines with cross-grain detail.
pub fn stripe(ctx: &NoiseContext, x: f64, y: f64, octaves: u32, angle_rad: f64) -> f64 {
    let rotated = x * angle_rad.cos() + y * angle_rad.sin();
    let grain = fbm(ctx, rotated * 3.0, 0.3, octaves, 2.0, 0.5);
    let cross = fbm(ctx, x * 0.5, y * 2.0, 2, 2.0, 0.5);
    grain * 0.8 + cross * 0.2
}

fn first_warp_pass(
    ctx: &NoiseContext, x: f64, y: f64,
    octaves: u32, strength: f64,
) -> (f64, f64) {
    let wx = fbm(ctx, x, y, octaves, 2.0, 0.5) * strength;
    let wy = fbm(ctx, x + 5.2, y + 1.3, octaves, 2.0, 0.5) * strength;
    (wx, wy)
}

fn second_warp_pass(
    ctx: &NoiseContext, x: f64, y: f64, wx: f64, wy: f64,
    octaves: u32, strength: f64,
) -> (f64, f64) {
    let wx2 = fbm(ctx, x + wx + 1.7, y + wy + 9.2, octaves, 2.0, 0.5) * strength;
    let wy2 = fbm(ctx, x + wx + 8.3, y + wy + 2.8, octaves, 2.0, 0.5) * strength;
    (wx2, wy2)
}
