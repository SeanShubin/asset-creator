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

impl Color3 {
    pub fn to_array(self) -> [f32; 3] {
        [
            self.0 as f32 / COLOR_DIVISOR,
            self.1 as f32 / COLOR_DIVISOR,
            self.2 as f32 / COLOR_DIVISOR,
        ]
    }

    pub fn to_rgb(self) -> (f32, f32, f32) {
        let a = self.to_array();
        (a[0], a[1], a[2])
    }

    pub fn from_array(a: [f32; 3]) -> Self {
        Self(
            (a[0] * COLOR_DIVISOR).round() as u8,
            (a[1] * COLOR_DIVISOR).round() as u8,
            (a[2] * COLOR_DIVISOR).round() as u8,
        )
    }
}

impl Default for Color3 {
    fn default() -> Self {
        Self(1, 1, 1) // ~33% grey
    }
}
