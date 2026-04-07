use noise::{NoiseFn, OpenSimplex, Perlin};

/// Pre-built noise generators for a given seed. Construct once, sample many times.
pub struct NoiseContext {
    perlin: Perlin,
    simplex: OpenSimplex,
    perlin_b: Perlin,
    simplex_b: OpenSimplex,
}

impl NoiseContext {
    pub fn new(seed: u32) -> Self {
        Self {
            perlin: Perlin::new(seed),
            simplex: OpenSimplex::new(seed),
            perlin_b: Perlin::new(seed.wrapping_add(137)),
            simplex_b: OpenSimplex::new(seed.wrapping_add(293)),
        }
    }

    pub fn perlin(&self, x: f64, y: f64) -> f64 {
        self.perlin.get([x, y])
    }

    pub fn simplex(&self, x: f64, y: f64) -> f64 {
        self.simplex.get([x, y])
    }

    pub fn perlin_b(&self, x: f64, y: f64) -> f64 {
        self.perlin_b.get([x, y])
    }

    pub fn simplex_b(&self, x: f64, y: f64) -> f64 {
        self.simplex_b.get([x, y])
    }
}
