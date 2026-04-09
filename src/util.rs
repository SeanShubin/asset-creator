use serde::{Deserialize, Serialize};

/// Parse a RON string with the project's standard options (implicit Some).
pub fn parse_ron<T: serde::de::DeserializeOwned>(ron_str: &str) -> Result<T, ron::error::SpannedError> {
    let options = ron::Options::default()
        .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
    options.from_str(ron_str)
}

/// RGB color with components in 0.0-1.0 range.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Color3(pub f32, pub f32, pub f32);

impl Color3 {
    pub fn to_array(self) -> [f32; 3] {
        [self.0, self.1, self.2]
    }

    pub fn from_array(a: [f32; 3]) -> Self {
        Self(a[0], a[1], a[2])
    }
}

impl Default for Color3 {
    fn default() -> Self {
        Self(0.5, 0.5, 0.5)
    }
}
