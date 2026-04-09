use noise::{NoiseFn, Perlin};

/// Pre-built noise generators for a given seed. Construct once, sample many times.
pub struct NoiseContext {
    perlin: Perlin,
    perlin_b: Perlin,
}

impl NoiseContext {
    pub fn new(seed: u32) -> Self {
        Self {
            perlin: Perlin::new(seed),
            perlin_b: Perlin::new(seed.wrapping_add(137)),
        }
    }

    pub fn perlin(&self, x: f64, y: f64) -> f64 {
        self.perlin.get([x, y])
    }

    pub fn perlin_b(&self, x: f64, y: f64) -> f64 {
        self.perlin_b.get([x, y])
    }
}
