use serde::{Deserialize, Serialize};

/// Parse a RON string with the project's standard options (implicit Some).
pub fn parse_ron<T: serde::de::DeserializeOwned>(ron_str: &str) -> Result<T, ron::error::SpannedError> {
    let options = ron::Options::default()
        .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
    options.from_str(ron_str)
}

/// RGB color with integer components in 0-3 range.
/// Converted to float via value / 3.0 (0=0%, 1=33%, 2=67%, 3=100%).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Color3(pub u8, pub u8, pub u8);

const COLOR_DIVISOR: f32 = 3.0;

/// Convert an sRGB component (0.0–1.0) to linear space.
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

impl Color3 {
    /// Convert to linear RGB for Bevy vertex colors. The integer
    /// 0–3 values are treated as sRGB (perceptually uniform), then
    /// converted to linear space for correct rendering.
    pub fn to_array(self) -> [f32; 3] {
        [
            srgb_to_linear(self.0 as f32 / COLOR_DIVISOR),
            srgb_to_linear(self.1 as f32 / COLOR_DIVISOR),
            srgb_to_linear(self.2 as f32 / COLOR_DIVISOR),
        ]
    }

    pub fn to_rgb(self) -> (f32, f32, f32) {
        let a = self.to_array();
        (a[0], a[1], a[2])
    }
}

impl Default for Color3 {
    fn default() -> Self {
        Self(1, 1, 1) // ~33% grey
    }
}
