> **Status:** Future / out-of-tree. The tileset editor was never implemented
> in this codebase; this document is preserved on 2026-04-19 as the design
> reference for a planned standalone tool. It depends on the surface concept,
> which was also moved to `docs/future/`.

# Tileset Editor

Generate 47-blob autotile tilesets with procedural surfaces, beveled lighting, and configurable zone geometry.

## Overview

The tileset editor produces complete tileset sheets matching the standard 47-blob autotile layout used by tile map editors (LDtk, Tiled, etc.). Each tile is rendered procedurally from [surface](surface-editor.md) definitions -- no hand-painted source textures are needed.

Each tile has up to three concentric zones, defined as fractions from outside in:

| Zone       | Surface  | Required | Description                                            |
| ---------- | -------- | -------- | ------------------------------------------------------ |
| **Outer**  | required | yes      | The exposed edge/ground area around the tile perimeter |
| **Middle** | optional | no       | Angled transition (bevel) between outer and inner      |
| **Inner**  | optional | no       | The raised/central face of the tile                    |

Zone fractions must sum to 1.0 (`outer + middle + inner = 1.0`).

## Zone Model

The 47-blob adjacency mask determines where each zone appears on each tile. On sides where the tile has adjacency (a neighbor of the same type), the outer zone is suppressed and the inner zone extends to that edge. On exposed sides (no neighbor), all three zones are visible in order: outer at the perimeter, then middle, then inner.

### Bevel Angle

The middle zone has a `bevel_angle` parameter (0-90 degrees) that controls the surface normal used for lighting:

- **0°** = flat. The middle zone is a flat band with a different surface than inner/outer. Normal points straight up. No lighting variation.
- **15-30°** = gentle ramp. Subtle lighting -- slightly lit on one side, slightly shadowed on the other.
- **45°** = classic bevel. The standard raised-tile look. Clear light/shadow distinction on the slope.
- **60°+** = steep slope. The middle zone receives very little light from above, appearing as a dark band.

The angle does not change the pixel width of the middle zone -- it only affects how light hits it. The per-pixel normal for the middle zone is:

```
normal = (sin(bevel_angle) * edge_direction, cos(bevel_angle))
```

Where `edge_direction` points from inner toward outer (away from the tile center on exposed edges).

The practical useful range is 0-60°. Beyond that the bevel is so dark it's essentially invisible. At 90° with zero middle fraction, the angle is irrelevant.

### Examples

**Flat ground tile** (no bevel):

```ron
(
    outer_fraction: 0.5,
    outer_surface: "ground",
    middle_fraction: 0.0,
    inner_fraction: 0.5,
    inner_surface: "grass",
)
```

A tile with N+S adjacency: the outer "ground" surface appears as 0.25-wide strips on the exposed left and right edges. The center is filled with "grass". No height transition.

**Classic raised tile** (beveled):

```ron
(
    outer_fraction: 0.0,
    middle_fraction: 0.5,
    middle_surface: "stone_slope",
    bevel_angle: 45.0,
    inner_fraction: 0.5,
    inner_surface: "stone_face",
)
```

A tile with N+S adjacency: the left and right edges have a 45° slope (lit/shadowed by the directional light), the center is flat "stone_face".

**Three-zone tile** (ground + bevel + face):

```ron
(
    outer_fraction: 0.25,
    outer_surface: "dirt",
    middle_fraction: 0.25,
    middle_surface: "stone_wall",
    bevel_angle: 50.0,
    inner_fraction: 0.5,
    inner_surface: "stone_floor",
)
```

## 47-Blob Autotile System

The 47-blob system encodes all possible configurations of a tile's 8 neighbors (N, NE, E, SE, S, SW, W, NW) into 47 distinct visual cases. Each case is stored at a fixed position in the tileset grid.

### Bitmask Encoding

Each neighbor direction maps to a bit:

| Direction | Bit | Value |
| --------- | --- | ----- |
| N         | 0   | 1     |
| NE        | 1   | 2     |
| E         | 2   | 4     |
| SE        | 3   | 8     |
| S         | 4   | 16    |
| SW        | 5   | 32    |
| W         | 6   | 64    |
| NW        | 7   | 128   |

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

## Lighting Model

The lighting model computes per-pixel brightness from a directional light source:

```
brightness = ambient + (1.0 - ambient) * max(0, dot(normal, light_dir))
```

The light direction is derived from `light_angle` (degrees, math convention: 0° = right, counter-clockwise positive):

```
light_dir = (cos(light_angle), sin(light_angle), overhead_z)
normalized
```

### Zone Normals

| Zone       | Normal                                          | Lighting behavior                                           |
| ---------- | ----------------------------------------------- | ----------------------------------------------------------- |
| **Inner**  | Straight up `(0, 0, 1)`                         | Uniform brightness across the flat face                     |
| **Middle** | Tilted by `bevel_angle` toward the exposed edge | Lit on the light-facing side, shadowed on the opposite side |
| **Outer**  | Straight up `(0, 0, 1)`                         | Uniform brightness (flat ground)                            |

Corner normals in the middle zone use radial distance for smooth curved transitions. Inner corners (concave notches) use inward-tilted normals.

### Parameters

| Parameter     | Type  | Default | Description                                                      |
| ------------- | ----- | ------- | ---------------------------------------------------------------- |
| `light_angle` | `f32` | 135.0   | Light direction in degrees (math convention). 135° = upper-left. |
| `ambient`     | `f32` | 0.25    | Minimum brightness floor (0.0-1.0)                               |

## Procedural Surfaces on Tiles

Each zone references a [surface](surface-editor.md) by name. Surfaces are evaluated using world-space tile coordinates (continuous across the full tileset sheet), so patterns tile seamlessly when tiles are placed adjacent in a map.

See [Surface Editor](surface-editor.md) for the full surface parameter reference.

## RON Format

```ron
(
    name: "stone_wall_tileset",
    outer_fraction: 0.0,
    middle_fraction: 0.25,
    middle_surface: "stone_slope",
    bevel_angle: 45.0,
    inner_fraction: 0.75,
    inner_surface: "stone_face",
    light_angle: 135.0,
    ambient: 0.25,
)
```

All fraction and angle fields have defaults (see parameter tables). Only non-default values need to be specified.

## Export

Tilesets can be exported to PNG at configurable tile resolutions:

```bash
# Default 64px tiles
cargo run -- tileset --export assets/generated/wall.png

# High-res 128px tiles
cargo run -- tileset --export wall_hd.png --tile-size 128

# With a specific tileset definition
cargo run -- tileset data/tilesets/stone_wall.tileset.ron --export stone.png
```

Export produces a gapless grid (no inter-tile spacing) suitable for direct use in tile map editors. The interactive editor adds 2px gaps between tiles for visual clarity.

## UI Panel

The egui side panel provides:
- Zone fraction sliders (outer, middle, inner -- constrained to sum to 1.0)
- Surface selectors for each zone (pick from named surfaces)
- Bevel angle slider (shown when middle fraction > 0)
- Lighting controls (angle, ambient)
- Export button with tile size input
