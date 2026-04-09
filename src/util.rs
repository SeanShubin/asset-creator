/// Parse a RON string with the project's standard options (implicit Some).
pub fn parse_ron<T: serde::de::DeserializeOwned>(ron_str: &str) -> Result<T, ron::error::SpannedError> {
    let options = ron::Options::default()
        .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
    options.from_str(ron_str)
}
