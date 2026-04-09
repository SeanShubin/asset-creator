use super::definition::{PatternType, SurfaceDef};
use crate::util::Color3;

pub fn preset_names() -> &'static [&'static str] {
    &[
        "Concrete",
        "Red Stone",
        "Dark Stone",
        "Marble",
        "Wood Plank",
        "Sandstone",
        "Metal Plate",
        "Brushed Metal",
        "Rusted Steel",
        "Dark Composite",
        "Energy",
    ]
}

pub fn preset_by_name(name: &str) -> Option<SurfaceDef> {
    let surface = match name {
        "Concrete" => concrete(),
        "Red Stone" => red_stone(),
        "Dark Stone" => dark_stone(),
        "Marble" => marble(),
        "Wood Plank" => wood_plank(),
        "Sandstone" => sandstone(),
        "Metal Plate" => metal_plate(),
        "Brushed Metal" => brushed_metal(),
        "Rusted Steel" => rusted_steel(),
        "Dark Composite" => dark_composite(),
        "Energy" => energy(),
        _ => return None,
    };
    Some(surface)
}

fn concrete() -> SurfaceDef {
    SurfaceDef {
        name: "concrete".into(),
        base_color: Color3(0.62, 0.62, 0.62),
        color_variation: Color3(0.06, 0.06, 0.06),
        noise_scale: 0.08,
        noise_octaves: 3,
        pattern: PatternType::Perlin,
        roughness: 0.7,
        ..Default::default()
    }
}

fn red_stone() -> SurfaceDef {
    SurfaceDef {
        name: "red_stone".into(),
        base_color: Color3(0.6, 0.25, 0.18),
        color_variation: Color3(0.06, 0.06, 0.06),
        noise_scale: 0.1,
        noise_octaves: 2,
        pattern: PatternType::Ridged,
        roughness: 0.8,
        speckle_density: 0.08,
        speckle_color: Color3(0.85, 0.85, 0.8),
        secondary_color: Some(Color3(0.78, 0.75, 0.65)),
        ..Default::default()
    }
}

fn dark_stone() -> SurfaceDef {
    SurfaceDef {
        name: "dark_stone".into(),
        base_color: Color3(0.3, 0.3, 0.32),
        color_variation: Color3(0.1, 0.1, 0.1),
        noise_scale: 0.04,
        noise_octaves: 4,
        pattern: PatternType::Cellular,
        roughness: 0.9,
        secondary_color: Some(Color3(0.18, 0.18, 0.2)),
        ..Default::default()
    }
}

fn marble() -> SurfaceDef {
    SurfaceDef {
        name: "marble".into(),
        base_color: Color3(0.88, 0.86, 0.82),
        color_variation: Color3(0.15, 0.15, 0.15),
        noise_scale: 0.03,
        noise_octaves: 4,
        pattern: PatternType::Marble,
        roughness: 0.3,
        secondary_color: Some(Color3(0.4, 0.35, 0.3)),
        ..Default::default()
    }
}

fn wood_plank() -> SurfaceDef {
    SurfaceDef {
        name: "wood_plank".into(),
        base_color: Color3(0.55, 0.38, 0.22),
        color_variation: Color3(0.12, 0.12, 0.12),
        noise_scale: 0.06,
        noise_octaves: 3,
        pattern: PatternType::Stripe,
        roughness: 0.6,
        stripe_angle: 0.0,
        secondary_color: Some(Color3(0.42, 0.28, 0.15)),
        ..Default::default()
    }
}

fn sandstone() -> SurfaceDef {
    SurfaceDef {
        name: "sandstone".into(),
        base_color: Color3(0.72, 0.62, 0.45),
        color_variation: Color3(0.1, 0.1, 0.1),
        noise_scale: 0.07,
        noise_octaves: 3,
        pattern: PatternType::Perlin,
        roughness: 0.7,
        speckle_density: 0.04,
        speckle_color: Color3(0.85, 0.78, 0.6),
        ..Default::default()
    }
}

fn metal_plate() -> SurfaceDef {
    SurfaceDef {
        name: "metal_plate".into(),
        base_color: Color3(0.5, 0.52, 0.55),
        color_variation: Color3(0.02, 0.02, 0.02),
        noise_scale: 0.2,
        noise_octaves: 1,
        pattern: PatternType::Perlin,
        roughness: 0.3,
        speckle_density: 0.02,
        speckle_color: Color3(0.7, 0.72, 0.75),
        ..Default::default()
    }
}

fn brushed_metal() -> SurfaceDef {
    SurfaceDef {
        name: "brushed_metal".into(),
        base_color: Color3(0.5, 0.5, 0.55),
        color_variation: Color3(0.05, 0.05, 0.06),
        noise_scale: 12.0,
        noise_octaves: 2,
        pattern: PatternType::Perlin,
        roughness: 0.3,
        ..Default::default()
    }
}

fn rusted_steel() -> SurfaceDef {
    SurfaceDef {
        name: "rusted_steel".into(),
        base_color: Color3(0.45, 0.3, 0.2),
        color_variation: Color3(0.15, 0.1, 0.05),
        noise_scale: 6.0,
        noise_octaves: 4,
        pattern: PatternType::Ridged,
        roughness: 0.8,
        ..Default::default()
    }
}

fn dark_composite() -> SurfaceDef {
    SurfaceDef {
        name: "dark_composite".into(),
        base_color: Color3(0.15, 0.15, 0.18),
        color_variation: Color3(0.03, 0.03, 0.04),
        noise_scale: 20.0,
        noise_octaves: 2,
        pattern: PatternType::Perlin,
        roughness: 0.4,
        ..Default::default()
    }
}

fn energy() -> SurfaceDef {
    SurfaceDef {
        name: "energy".into(),
        base_color: Color3(0.1, 0.4, 0.8),
        color_variation: Color3(0.2, 0.3, 0.1),
        noise_scale: 4.0,
        noise_octaves: 3,
        pattern: PatternType::Perlin,
        roughness: 0.1,
        ..Default::default()
    }
}
