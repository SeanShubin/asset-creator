use super::base::NoiseContext;

/// Fractal Brownian Motion: layers octaves of Perlin noise.
pub fn fbm(ctx: &NoiseContext, x: f64, y: f64, octaves: u32, lacunarity: f64, gain: f64) -> f64 {
    accumulate_octaves(ctx, x, y, octaves, lacunarity, gain, |val| val)
}

/// Ridged multifractal: sharp ridges from inverted absolute values.
pub fn ridged(ctx: &NoiseContext, x: f64, y: f64, octaves: u32, lacunarity: f64, gain: f64) -> f64 {
    let mut value = 0.0;
    let mut amplitude = 1.0;
    let mut frequency = 1.0;
    let mut weight = 1.0;

    for _ in 0..octaves {
        let signal = (1.0 - ctx.perlin(x * frequency, y * frequency).abs()) * amplitude;
        let weighted = signal * weight;
        weight = (weighted * 2.0).clamp(0.0, 1.0);
        value += weighted;
        amplitude *= gain;
        frequency *= lacunarity;
    }

    value * 0.5 - 0.5
}

/// Turbulence: absolute-value FBM producing billowy patterns.
pub fn turbulence(ctx: &NoiseContext, x: f64, y: f64, octaves: u32, lacunarity: f64, gain: f64) -> f64 {
    let result = accumulate_octaves(ctx, x, y, octaves, lacunarity, gain, |val| val.abs());
    result * 2.0 - 1.0
}

fn accumulate_octaves(
    ctx: &NoiseContext,
    x: f64,
    y: f64,
    octaves: u32,
    lacunarity: f64,
    gain: f64,
    transform: impl Fn(f64) -> f64,
) -> f64 {
    let mut value = 0.0;
    let mut amplitude = 1.0;
    let mut frequency = 1.0;
    let mut max_amp = 0.0;

    for _ in 0..octaves {
        value += transform(ctx.perlin(x * frequency, y * frequency)) * amplitude;
        max_amp += amplitude;
        amplitude *= gain;
        frequency *= lacunarity;
    }

    value / max_amp
}
