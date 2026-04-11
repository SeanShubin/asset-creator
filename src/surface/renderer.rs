use super::definition::{PatternType, SurfaceDef};
use crate::noise::{self, NoiseContext};


/// Renders a surface definition to RGBA pixel data.
pub fn render_surface(surface: &SurfaceDef, width: u32, height: u32) -> Vec<u8> {
    let ctx = NoiseContext::new(surface.seed);
    let mut pixels = vec![0u8; (width * height * 4) as usize];

    for py in 0..height {
        for px in 0..width {
            let (nx, ny) = pixel_to_noise_coords(px, py, width, height, surface.noise_scale);
            let color = sample_surface(&ctx, surface, nx, ny, px as f64, py as f64);
            write_pixel(&mut pixels, px, py, width, color);
        }
    }

    pixels
}

fn sample_surface(ctx: &NoiseContext, surface: &SurfaceDef, nx: f64, ny: f64, px: f64, py: f64) -> [f32; 3] {
    let noise_val = evaluate_pattern(ctx, &surface.pattern, nx, ny, surface);
    let base = apply_color_variation(surface, noise_val);
    let with_speckle = apply_speckle(ctx, surface, base, px, py);
    clamp_color(with_speckle)
}

fn pixel_to_noise_coords(px: u32, py: u32, width: u32, height: u32, scale: f32) -> (f64, f64) {
    let nx = px as f64 / width as f64 * scale as f64;
    let ny = py as f64 / height as f64 * scale as f64;
    (nx, ny)
}

fn evaluate_pattern(ctx: &NoiseContext, pattern: &PatternType, x: f64, y: f64, surface: &SurfaceDef) -> f64 {
    let octaves = surface.noise_octaves;

    match pattern {
        PatternType::Perlin => noise::fbm(ctx, x, y, octaves, 2.0, 0.5),
        PatternType::Cellular => noise::cellular2d(x, y, surface.seed),
        PatternType::Ridged => noise::ridged(ctx, x, y, octaves, 2.0, 0.5),
        PatternType::Stripe => {
            let angle_rad = (surface.stripe_angle as f64).to_radians();
            noise::stripe(ctx, x, y, octaves, angle_rad)
        }
        PatternType::Marble => noise::marble(ctx, x, y, octaves, 4.0),
        PatternType::Turbulence => noise::turbulence(ctx, x, y, octaves, 2.0, 0.5),
        PatternType::DomainWarp => noise::domain_warp(ctx, x, y, octaves, 4.0),
    }
}

fn apply_color_variation(surface: &SurfaceDef, noise_val: f64) -> [f32; 3] {
    let [br, bg, bb] = surface.base_color.to_array();
    let [vr, vg, vb] = surface.color_variation.to_array();
    let n = noise_val as f32;

    if let Some(secondary) = surface.secondary_color {
        let [sr, sg, sb] = secondary.to_array();
        let blend = (n * 0.5 + 0.5).clamp(0.0, 1.0);
        [
            lerp(br, sr, blend) + vr * n,
            lerp(bg, sg, blend) + vg * n,
            lerp(bb, sb, blend) + vb * n,
        ]
    } else {
        [br + vr * n, bg + vg * n, bb + vb * n]
    }
}

fn apply_speckle(ctx: &NoiseContext, surface: &SurfaceDef, color: [f32; 3], px: f64, py: f64) -> [f32; 3] {
    if noise::speckle(ctx, px, py, surface.speckle_density as f64) {
        surface.speckle_color.to_array()
    } else {
        color
    }
}

fn clamp_color(color: [f32; 3]) -> [f32; 3] {
    [
        color[0].clamp(0.0, 1.0),
        color[1].clamp(0.0, 1.0),
        color[2].clamp(0.0, 1.0),
    ]
}

fn write_pixel(pixels: &mut [u8], px: u32, py: u32, width: u32, color: [f32; 3]) {
    let idx = ((py * width + px) * 4) as usize;
    pixels[idx] = (color[0] * 255.0) as u8;
    pixels[idx + 1] = (color[1] * 255.0) as u8;
    pixels[idx + 2] = (color[2] * 255.0) as u8;
    pixels[idx + 3] = 255;
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
