use super::base::NoiseContext;

/// Deterministic pseudo-random value in [0, 1) from integer coordinates.
pub fn hash2d(x: i32, y: i32, seed: u32) -> f64 {
    let mut h = seed;
    h = h.wrapping_add(x as u32).wrapping_mul(0x9E37_79B9);
    h = h.wrapping_add(y as u32).wrapping_mul(0x517C_C1B7);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    (h & 0x7FFF_FFFF) as f64 / 0x7FFF_FFFF as f64
}

/// Returns true if this position should receive a speckle dot.
pub fn speckle(ctx: &NoiseContext, x: f64, y: f64, density: f64) -> bool {
    if density <= 0.0 {
        return false;
    }
    let hash = ctx.perlin_b(x * 1.731, y * 2.399);
    hash > 1.0 - density * 2.0
}
