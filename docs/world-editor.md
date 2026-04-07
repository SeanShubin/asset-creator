# World Editor

Interactive biome-based terrain generation with noise-driven layers for elevation, moisture, and drainage.

## Overview

The world editor generates multi-biome terrain maps using layered noise fields. Three independent noise channels (elevation, moisture, drainage) are combined to determine biome type at each point, with smooth blending at biome boundaries and optional hill-shaded lighting.

## Noise Layers

The terrain is driven by three independent noise fields, each using fractal Brownian motion (fBM) with configurable parameters:

| Layer | Controls | Typical Frequency |
|-------|----------|-------------------|
| **Elevation** | Water vs. land vs. mountain | 3.0 |
| **Moisture** | Dry vs. wet biomes on land | 4.0 |
| **Drainage** | Swamp/marsh vs. grassland/forest | 5.0 |

Each layer has its own seed offset for independence, and all share the same fBM parameters:
- **Octaves** (1-10): Detail level. More octaves add finer features.
- **Lacunarity** (1.0-4.0): Frequency multiplier between octaves. Higher values add more high-frequency detail.
- **Gain / Persistence** (0.1-0.9): Amplitude multiplier between octaves. Lower values make large features dominate.

## Biome System

Biomes are determined by a lookup function mapping `(elevation, moisture, drainage)` to a biome type:

### Elevation-Driven Biomes

| Elevation Range | Biome |
|----------------|-------|
| 0.00 - 0.30 | Deep Water |
| 0.30 - 0.38 | Shallow Water |
| 0.38 - 0.42 | Beach |
| 0.75 - 0.85 | Rock |
| 0.85 - 1.00 | Snow |

### Land Biomes (elevation 0.42 - 0.75)

Land biomes are determined by moisture and drainage:

| | Low Drainage (<0.35) | Mid Drainage (0.35-0.65) | High Drainage (>0.65) |
|---|---|---|---|
| **Dry** (moisture <0.25) | Desert | Desert | Savanna |
| **Moderate** (0.25-0.55) | Marsh | Grassland | Grassland |
| **Wet** (>0.55) | Swamp | Forest | Dense Forest |

### Biome Colors

Each biome has a base color and a detail color, blended by a detail noise layer to add micro-variation:

| Biome | Base Color | Detail Color |
|-------|-----------|--------------|
| Deep Water | Dark blue | Darker blue |
| Shallow Water | Medium blue | Slightly darker blue |
| Beach | Tan | Warm sand |
| Desert | Yellow-brown | Darker yellow |
| Savanna | Olive | Dark olive |
| Grassland | Green | Darker green |
| Forest | Dark green | Very dark green |
| Dense Forest | Very dark green | Near-black green |
| Swamp | Dark olive | Brown-green |
| Marsh | Gray-green | Darker gray-green |
| Rock | Gray-brown | Darker gray |
| Snow | Near-white | Light gray |

## Biome Blending

At biome boundaries, colors are smoothly blended using stochastic sampling:

1. For each pixel, sample the biome function at multiple jittered offsets within a `blend_width` radius in parameter space (elevation/moisture/drainage)
2. Average the resulting colors across all samples
3. This creates smooth, natural-looking transitions without hard edges

### Blend Parameters

| Parameter | Range | Default | Description |
|-----------|-------|---------|-------------|
| `blend_width` | 0.0-0.2 | 0.06 | Radius of blending in parameter space |
| `blend_samples` | 1-16 | 8 | Number of jittered samples for blending |

Setting `blend_width` to 0 or `blend_samples` to 1 produces hard biome boundaries.

The jitter uses golden-ratio spacing for even distribution: offsets are placed at angles `i/n * TAU` with radii `blend_width * fract(i * 0.618034)`.

## Detail Noise

A separate high-frequency noise layer adds micro-variation within biomes, blending between each biome's base and detail colors:

| Parameter | Range | Default | Description |
|-----------|-------|---------|-------------|
| `detail_strength` | 0.0-1.0 | 0.3 | Amplitude of detail variation |
| `detail_freq` | 5.0-60.0 | 20.0 | Spatial frequency of detail noise |

## Hill-Shaded Lighting

The "Biomes (lit)" view mode computes per-pixel normals from the elevation field and applies directional lighting:

### Normal Computation

For each pixel, the surface normal is derived from the elevation gradient:
```
dh/dx = (elevation[x+1] - elevation[x-1]) / (2 * step)
dh/dy = (elevation[y-1] - elevation[y+1]) / (2 * step)
normal = normalize(-dh/dx * height_scale, -dh/dy * height_scale, 1.0)
```

### Lighting Parameters

| Parameter | Range | Default | Description |
|-----------|-------|---------|-------------|
| `light_azimuth` | 0-360 degrees | 225 | Direction the light comes from |
| `light_elevation` | 5-85 degrees | 45 | Angle of light above the horizon |
| `ambient` | 0.0-0.5 | 0.15 | Minimum brightness in shadow |
| `height_scale` | 0.1-5.0 | 1.0 | Exaggeration of terrain relief |

Shading formula: `brightness = ambient + max(0, dot(normal, light_dir))`

## View Modes

| Mode | Description |
|------|-------------|
| **Biomes (lit)** | Full biome colors with hill-shaded lighting |
| **Biomes (flat)** | Biome colors without lighting |
| **Elevation** | Grayscale elevation map |
| **Moisture** | Blue-scale moisture map |
| **Drainage** | Green-scale drainage map |
| **Blend Weights** | RGB visualization (R=elevation, G=moisture, B=drainage) |

## RON Format

World definitions specify noise parameters and biome configuration:

```ron
(
    seed: 42,
    elevation_freq: 3.0,
    moisture_freq: 4.0,
    drainage_freq: 5.0,
    octaves: 6,
    lacunarity: 2.0,
    gain: 0.5,
    blend_width: 0.06,
    blend_samples: 8,
    detail_strength: 0.3,
    detail_freq: 20.0,
    light_azimuth: 225.0,
    light_elevation: 45.0,
    ambient: 0.15,
    height_scale: 1.0,
)
```

## UI Panel

The egui side panel provides:
- **View mode** selector (6 modes)
- **Seed** input for reproducible generation
- **Per-layer frequency** sliders (elevation, moisture, drainage)
- **fBM parameters** (octaves, lacunarity, gain)
- **Blending controls** (width, sample count)
- **Detail noise** (strength, frequency)
- **Lighting controls** (azimuth, elevation, ambient, height scale) -- shown only in lit mode

All parameters regenerate the terrain texture immediately on change.

## Command Line

```bash
# Interactive world editor
cargo run -- world

# Load world definition
cargo run -- world data/worlds/archipelago.ron

# Export world map as PNG
cargo run -- world --seed 42 --export assets/generated/world.png
```
