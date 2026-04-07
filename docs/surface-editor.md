# Surface Editor

Define the visual appearance of a surface once, then apply it to any output -- 2D pixel textures, 3D shader materials, tileset faces, or world terrain.

## Overview

A **surface** is the abstract visual definition of how something looks: its color, noise pattern, roughness, and detail. It is output-agnostic. The same surface definition can be:

- Rendered as CPU-side pixels for tileset sheets and 2D sprite exports
- Compiled into a WGSL fragment shader for real-time 3D object materials
- Referenced by name from tileset face/edge zones, object parts, and world biomes

This separation means you design the look first, then decide where it goes.

## Surface Definition

A surface is defined by these parameters:

```ron
(
    name: "rusted_steel",
    base_color: (0.45, 0.3, 0.2),
    color_variation: (0.15, 0.1, 0.05),
    noise_scale: 6.0,
    noise_octaves: 4,
    pattern: "Ridged",
    roughness: 0.8,
    speckle_density: 0.0,
    speckle_color: (1.0, 1.0, 1.0),
    secondary_color: Some((0.35, 0.22, 0.15)),
    stripe_angle: 90.0,
    seed: 42,
)
```

### Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `name` | `String` | required | Identifier for referencing this surface from other assets |
| `base_color` | `(f32, f32, f32)` | `(0.5, 0.5, 0.55)` | Primary RGB color (0.0-1.0) |
| `color_variation` | `(f32, f32, f32)` | `(0.08, 0.06, 0.04)` | Per-channel noise-driven color offset |
| `noise_scale` | `f32` | `8.0` | Spatial frequency of the noise pattern |
| `noise_octaves` | `u32` | `3` | FBM octaves (1 = smooth, 4+ = detailed) |
| `pattern` | `PatternType` | `"Perlin"` | Which noise function drives the look |
| `roughness` | `f32` | `0.6` | PBR roughness (0.0 = mirror, 1.0 = matte). Used by 3D output; ignored by 2D pixel export. |
| `speckle_density` | `f32` | `0.0` | Fraction of pixels that receive speckle dots (0.0-1.0) |
| `speckle_color` | `(f32, f32, f32)` | `(1.0, 1.0, 1.0)` | Color of speckle dots |
| `secondary_color` | `Option<(f32, f32, f32)>` | `None` | For two-tone patterns; blended with base via noise |
| `stripe_angle` | `f32` | `90.0` | Angle in degrees for stripe patterns (math convention: 0° = horizontal, 90° = vertical) |
| `seed` | `u32` | `42` | Noise seed for reproducibility |

### Pattern Types

| Pattern | Description |
|---------|-------------|
| `Perlin` | Smooth, organic bumpy surface from Perlin noise |
| `Cellular` | Voronoi/Worley-style cell boundaries (F2-F1) |
| `Ridged` | Ridged multifractal -- sharp mountain-like ridges |
| `Stripe` | Directional lines with cross-grain detail, controlled by `stripe_angle` |
| `Marble` | Domain-warped Perlin passed through a sine function, producing veined stone |
| `Turbulence` | Absolute-value FBM producing billowy, cloud-like patterns |
| `DomainWarp` | Multi-pass domain warping for organic, flowing distortions |

See [Noise Functions](noise-functions.md) for the full math behind each pattern.

## Presets

Built-in presets provide starting points:

| Preset | Base Color | Pattern | Roughness | Character |
|--------|-----------|---------|-----------|-----------|
| Concrete | Gray | Perlin (3 octave) | 0.7 | Rough, granular surface |
| Red Stone | Brick red | Ridged + speckles | 0.8 | Rough stone with flecks |
| Dark Stone | Dark gray | Cellular (4 octave) | 0.9 | Cracked dark rock |
| Marble | Off-white | Marble (domain warp) | 0.3 | Veined polished stone |
| Wood Plank | Warm brown | Stripe (horizontal) | 0.6 | Directional grain |
| Sandstone | Tan | Perlin + speckles | 0.7 | Warm granular stone |
| Metal Plate | Silver | Perlin (1 octave) | 0.3 | Smooth brushed metal |
| Brushed Metal | Silver | Fine Perlin | 0.3 | Subtle directional grain |
| Rusted Steel | Orange-brown | Ridged (high strength) | 0.8 | Heavy corrosion |
| Dark Composite | Near-black | Fine Perlin | 0.4 | Carbon fiber feel |
| Energy | Blue | Coarse Perlin | 0.1 | Glowing, shifting field |

## How Surfaces Are Used

### In 3D Objects

Object nodes reference surfaces by name. The surface is compiled into a custom WGSL shader material evaluated in world space -- no UV mapping needed:

```ron
// shape definition
(name: "chassis", shape: Box(size: (1.4, 0.5, 0.8)),
 surface: "rusted_steel")
```

The shader receives world-space position from the vertex stage and evaluates the noise function to perturb the base color. The `roughness` parameter maps directly to PBR roughness. The pattern flows seamlessly across all mesh parts sharing the same surface.

#### Shader Architecture

Surfaces are implemented as Bevy `Material` types with `AsBindGroup`:

```rust
#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct SurfaceMaterial {
    #[uniform(0)]
    base_color: LinearRgba,
    #[uniform(0)]
    noise_scale: f32,
    #[uniform(0)]
    noise_strength: f32,
    #[uniform(0)]
    color_variation: LinearRgba,
    #[uniform(0)]
    roughness: f32,
}

impl Material for SurfaceMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/surface.wgsl".into()
    }
}
```

### In Tilesets

Tileset face and edge zones each reference a surface. The surface is evaluated as CPU-side pixels using the same noise functions:

```ron
// tileset definition
(
    face_surface: "concrete",
    edge_surface: "dark_stone",
    edge_fraction: 0.22,
    light_angle: 135.0,
)
```

The noise is sampled in world-space tile coordinates so the pattern is continuous across adjacent tiles. The `roughness` parameter is not used for 2D output.

See [Tileset Editor](tileset-editor.md) for the full tileset reference.

### In World Terrain

Biome definitions can reference surfaces for their ground appearance:

```ron
// biome override (future)
grassland_surface: "grass_blend",
```

### As Standalone Export

A surface can be rendered to a PNG image directly:

```bash
cargo run -- surface --preset Marble --export marble_512.png --size 512
```

## 2D vs 3D Rendering

The same surface definition drives both rendering paths. The core difference is where the noise is evaluated:

| | 2D Pixel Output | 3D Shader Output |
|---|---|---|
| **Evaluator** | CPU (Rust noise crate) | GPU (WGSL shader) |
| **Coordinate space** | Pixel coordinates / world tile coords | World-space vertex position |
| **Output** | RGBA pixel buffer / PNG | Fragment color + PBR roughness |
| **Roughness** | Ignored (lighting is baked or N/A) | Maps to PBR roughness |
| **Use cases** | Tilesets, sprites, exported images | 3D objects, live preview |

The visual result is the same -- the noise math is identical, just executed on different hardware.

## UI Panel

The surface editor provides real-time editing with preview in both 2D and 3D:

- **Preset selector** with keyboard shortcuts (1-5)
- **Color pickers** for base color and color variation
- **Noise sliders**: scale, octave count, pattern type
- **Roughness slider** (visible in 3D preview mode)
- **Speckle controls**: density and color
- **Secondary color** toggle and picker
- **Stripe angle** (shown when pattern is Stripe)
- **Seed input** for reproducibility
- **Preview mode toggle**: 2D texture swatch / 3D shape preview
- All changes apply immediately

## RON File Format

Surfaces use `.surface.ron` extension:

```ron
// surfaces/rusted_steel.surface.ron
(
    name: "rusted_steel",
    base_color: (0.45, 0.3, 0.2),
    color_variation: (0.15, 0.1, 0.05),
    noise_scale: 6.0,
    noise_octaves: 4,
    pattern: "Ridged",
    roughness: 0.8,
    seed: 42,
)
```

Fields use `#[serde(default)]` so only non-default values need to be specified. The defaults above produce a neutral gray brushed-metal appearance.

## Command Line

```bash
# Interactive surface editor
cargo run -- surface

# Load a specific surface
cargo run -- surface data/surfaces/rusted_steel.surface.ron

# Load a preset
cargo run -- surface --preset Marble

# Export surface as 2D image
cargo run -- surface --preset Concrete --export concrete.png --size 512

# Preview surface on 3D shapes
cargo run -- surface --preset Energy --3d
```
