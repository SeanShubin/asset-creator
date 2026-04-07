# Tileset Editor

Generate 47-blob autotile tilesets with procedural surfaces, beveled 3D lighting, and multiple tile styles.

## Overview

The tileset editor produces complete tileset sheets matching the standard 47-blob autotile layout used by tile map editors (LDtk, Tiled, etc.). Each tile is rendered procedurally from surface definitions -- no hand-painted source textures are needed.

The editor supports two tile styles:
- **Bevel**: Elevated tiles with 3D-lit beveled edges
- **Ground**: Flat tiles with a border/outline texture

Both styles use independent face and edge [surfaces](surface-editor.md).

## 47-Blob Autotile System

The 47-blob system encodes all possible configurations of a tile's 8 neighbors (N, NE, E, SE, S, SW, W, NW) into 47 distinct visual cases. Each case is stored at a fixed position in the tileset grid.

### Bitmask Encoding

Each neighbor direction maps to a bit:

| Direction | Bit | Value |
|-----------|-----|-------|
| N | 0 | 1 |
| NE | 1 | 2 |
| E | 2 | 4 |
| SE | 3 | 8 |
| S | 4 | 16 |
| SW | 5 | 32 |
| W | 6 | 64 |
| NW | 7 | 128 |

Corner bits (NE, SE, SW, NW) are only relevant when both adjacent cardinal neighbors are present. This reduces the full 256 combinations to 47 unique visual cases.

### Grid Layout

Tiles are arranged in a 12x5 grid (with gaps for the interactive editor, no gaps for export). The layout follows the standard LDtk blob convention:

- Columns 0-4: Primary tiles (outer corners, edges, full connectivity)
- Columns 5-11: Secondary tiles (T-junctions, corridors, caps, diagonals)
- Position `(3, 1)` = isolated tile (mask 0, no neighbors)
- Position `(1, 1)` = fully connected tile (mask 255, all neighbors)

### Edge Detection

For each tile, the bitmask determines which edges are exposed:

- **Cardinal edges** (N, S, E, W): Direct neighbor is absent
- **Outer corners**: Cardinal neighbor absent, making the diagonal corner exposed
- **Inner corners**: Both adjacent cardinals are present, but the diagonal is absent -- producing a concave notch

## Bevel Lighting Model

Beveled tiles simulate a 3D raised surface using per-pixel normal computation:

### Geometry

Each tile has three zones:
1. **Face**: The flat top surface (normal pointing straight up: `[0, 0, 1]`)
2. **Bevel**: Angled slope from face to edge, width controlled by `edge_fraction` (default 0.22 = 22% of tile size)
3. **Edge**: The vertical side (only visible in ground-style as a border)

### Lighting

The lighting model computes per-pixel brightness from a directional light:

```
brightness = ambient + (1.0 - ambient) * max(0, dot(normal, light_dir))
```

Parameters:
- `light_angle`: Direction of the light source in degrees (default: 135, from upper-left)
- `shadow_strength`: How dark the shadowed bevels get (0.0-1.0)
- `highlight_strength`: How bright the lit bevels get (0.0-1.0)
- `ambient`: Minimum brightness floor (default: 0.25)

### Bevel Normal Computation

For each pixel in the bevel zone, the surface normal is computed based on:
- Distance to the nearest edge
- Bevel angle: `atan(bevel_depth / bevel_width)`
- The normal is tilted away from the face toward the edge direction

Corner bevels use radial distance for smooth curved transitions. Inner corners produce concave geometry with inverted normals.

### Edge Lines

Optional edge lines mark bevel boundaries:
- **Outer line**: Lighter, at the top of the bevel ridge
- **Inner line**: Darker, where the bevel meets the face
- Lines are suppressed at tile boundaries (where adjacent tiles would connect)

## Procedural Surfaces on Tiles

Face and edge zones each reference a [surface](surface-editor.md) definition. The surface is sampled using world-space coordinates (continuous across the full tileset sheet), so patterns tile seamlessly when tiles are placed adjacent in a map.

Surface parameters per zone include base color, color variation, noise scale, octave count, pattern type, speckle overlay, and secondary color. See [Surface Editor](surface-editor.md) for the full parameter reference.

## Presets

| Preset | Style | Description |
|--------|-------|-------------|
| Beveled Block | Bevel | Flat gray with 3D lighting and edge lines, no texture |
| Concrete | Bevel | Gray Perlin noise with moderate detail |
| Red Stone | Bevel | Ridged pattern with brick-like speckles |
| Dark Stone | Bevel | High-octave cellular pattern |
| Marble | Bevel | Domain-warped veined stone |
| Wood Plank | Bevel | Horizontal stripe grain |
| Sandstone | Bevel | Warm Perlin with fine speckles |
| Metal Plate | Bevel | Low-variation, high-bevel-depth metal |

## RON Format

Tileset definitions can be specified in RON:

```ron
(
    style: Bevel,
    face_surface: "concrete",
    edge_surface: "dark_concrete",
    edge_fraction: 0.22,
    light_angle: 135.0,
    shadow_strength: 0.7,
    highlight_strength: 0.4,
)
```

Surfaces can also be defined inline:

```ron
(
    style: Bevel,
    face_surface: (
        base_color: (0.62, 0.62, 0.62),
        color_variation: (0.06, 0.06, 0.06),
        noise_scale: 0.08,
        noise_octaves: 3,
        pattern: "Perlin",
        seed: 42,
    ),
    edge_surface: (
        base_color: (0.50, 0.50, 0.55),
        color_variation: (0.03, 0.03, 0.03),
        noise_scale: 0.08,
        noise_octaves: 2,
        pattern: "Perlin",
        seed: 42,
    ),
    edge_fraction: 0.22,
    light_angle: 135.0,
    shadow_strength: 0.7,
    highlight_strength: 0.4,
)
```

## Export

Tilesets can be exported to PNG at configurable tile resolutions:

```bash
# Default 64px tiles
cargo run -- tileset --export assets/generated/wall.png

# High-res 128px tiles
cargo run -- tileset --export wall_hd.png --tile-size 128

# With preset
cargo run -- tileset --preset DarkStone --export dark_stone.png
```

Export produces a gapless grid (no inter-tile spacing) suitable for direct use in tile map editors. The interactive editor adds 2px gaps between tiles for visual clarity.

## UI Panel

The egui side panel provides:
- Tile style selector (Bevel / Ground)
- Face surface parameters (full surface config, or pick from named surfaces)
- Edge surface parameters (independent config)
- Edge fraction slider
- Bevel lighting controls (angle, shadow, highlight, ambient)
- 3D lighting toggle
- Edge line toggle
- Preset selector
- Export button with tile size input
