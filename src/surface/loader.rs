use super::definition::SurfaceDef;
use std::path::Path;

pub fn load_surface_from_file(path: &Path) -> Result<SurfaceDef, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read '{}': {}", path.display(), e))?;
    parse_surface_ron(&contents)
}

fn parse_surface_ron(ron_str: &str) -> Result<SurfaceDef, String> {
    ron::de::from_str(ron_str)
        .map_err(|e| format!("Failed to parse RON: {}", e))
}
