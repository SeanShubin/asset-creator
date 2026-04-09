use serde::{Deserialize, Serialize};
use crate::util::Color3;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PatternType {
    Perlin,
    Cellular,
    Ridged,
    Stripe,
    Marble,
    Turbulence,
    DomainWarp,
}

impl Default for PatternType {
    fn default() -> Self {
        Self::Perlin
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceDef {
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default = "default_base_color")]
    pub base_color: Color3,
    #[serde(default = "default_color_variation")]
    pub color_variation: Color3,
    #[serde(default = "default_noise_scale")]
    pub noise_scale: f32,
    #[serde(default = "default_noise_octaves")]
    pub noise_octaves: u32,
    #[serde(default)]
    pub pattern: PatternType,
    #[serde(default = "default_roughness")]
    pub roughness: f32,
    #[serde(default)]
    pub speckle_density: f32,
    #[serde(default = "default_speckle_color")]
    pub speckle_color: Color3,
    #[serde(default)]
    pub secondary_color: Option<Color3>,
    #[serde(default = "default_stripe_angle")]
    pub stripe_angle: f32,
    #[serde(default = "default_seed")]
    pub seed: u32,
}

impl Default for SurfaceDef {
    fn default() -> Self {
        Self {
            name: default_name(),
            base_color: default_base_color(),
            color_variation: default_color_variation(),
            noise_scale: default_noise_scale(),
            noise_octaves: default_noise_octaves(),
            pattern: PatternType::default(),
            roughness: default_roughness(),
            speckle_density: 0.0,
            speckle_color: default_speckle_color(),
            secondary_color: None,
            stripe_angle: default_stripe_angle(),
            seed: default_seed(),
        }
    }
}

fn default_name() -> String {
    "unnamed".into()
}
fn default_base_color() -> Color3 {
    Color3(0.5, 0.5, 0.55)
}
fn default_color_variation() -> Color3 {
    Color3(0.08, 0.06, 0.04)
}
fn default_noise_scale() -> f32 {
    8.0
}
fn default_noise_octaves() -> u32 {
    3
}
fn default_roughness() -> f32 {
    0.6
}
fn default_speckle_color() -> Color3 {
    Color3(1.0, 1.0, 1.0)
}
fn default_stripe_angle() -> f32 {
    90.0
}
fn default_seed() -> u32 {
    42
}
