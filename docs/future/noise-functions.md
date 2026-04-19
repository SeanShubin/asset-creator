> **Status:** Future / out-of-tree. The noise module was deleted on
> 2026-04-19 together with the surface editor — nothing in the current
> codebase consumes it. This document is preserved as a design reference for
> planned standalone tools (surface, tileset, decal, world editors) that
> would share a noise library.

# Noise Functions

Procedural noise primitives used across all editors for texture generation, terrain, material variation, and detail.

## Overview

Noise functions are the foundation of all procedural content in the application. They transform spatial coordinates into pseudo-random values with controllable frequency, detail level, and character. The same noise library is shared by the texture editor, tileset generator, material system, and world editor.

## Base Noise Types

### Simplex Noise

Gradient noise on a simplex grid. Slightly faster than Perlin with fewer directional artifacts.

- Range: approximately -1.0 to 1.0
- Isotropic (no axis-aligned bias)
- Smooth, organic appearance

### Perlin Noise

Classic gradient noise on a regular grid.

- Range: approximately -1.0 to 1.0
- Slight grid-aligned artifacts at low frequencies
- The most commonly used base noise

## Composite Noise Functions

### Fractal Brownian Motion (fBM)

Layers multiple octaves of a base noise at increasing frequency and decreasing amplitude:

```
value = 0
amplitude = 1.0
frequency = 1.0
for each octave:
    value += noise(x * frequency, y * frequency) * amplitude
    amplitude *= gain        // typically 0.5
    frequency *= lacunarity  // typically 2.0
normalize by sum of amplitudes
```

Parameters:
- **Octaves** (1-10): Number of noise layers. More octaves = more fine detail.
- **Lacunarity** (1.0-4.0): Frequency multiplier per octave. 2.0 is standard (each octave is twice the frequency).
- **Gain / Persistence** (0.1-0.9): Amplitude multiplier per octave. 0.5 means each octave contributes half as much as the previous.

Character: Smooth, natural-looking variation. The go-to noise for most surfaces.

### Ridged Multifractal

Modified fBM where each octave's output is inverted through `1.0 - abs(signal)`, producing sharp ridges:

```
signal = (1.0 - abs(noise(x * freq, y * freq))) * amplitude
weight = clamp(signal * 2.0, 0.0, 1.0)
```

Each octave is weighted by the previous octave's output, creating detail concentration at ridge peaks.

Character: Mountain ridges, cracked surfaces, lightning patterns.

### Turbulence

fBM with absolute values, producing billowy, cloud-like patterns:

```
value += abs(noise(x * frequency, y * frequency)) * amplitude
```

Character: Smoke, clouds, marble veining base.

### Domain Warp

Multi-pass noise where the output of one noise evaluation offsets the input of the next:

```
wx = fbm(x, y) * warp_strength
wy = fbm(x + 5.2, y + 1.3) * warp_strength
wx2 = fbm(x + wx + 1.7, y + wy + 9.2) * warp_strength
wy2 = fbm(x + wx + 8.3, y + wy + 2.8) * warp_strength
result = fbm(x + wx2, y + wy2)
```

The magic constants (5.2, 1.3, etc.) break symmetry between passes.

Parameters:
- **warp_strength** (0.1-10.0): How far the domain is distorted

Character: Organic, flowing, water-stain-like patterns. Excellent for alien landscapes and weathering.

### Marble

Sine wave driven by domain-warped noise:

```
warp = fbm(x, y) * warp_strength
result = sin((x + warp) * PI)
```

Character: Veined stone, wood grain, flowing lines.

### Voronoi / Worley Noise

Grid-based cellular noise. Random points are placed in each grid cell, and the output is derived from distances to the nearest points.

Distance metrics:
- **Euclidean**: `sqrt(dx^2 + dy^2)` -- natural, circular cells
- **Manhattan**: `|dx| + |dy|` -- diamond-shaped cells
- **Chebyshev**: `max(|dx|, |dy|)` -- square cells

Output modes:
- **F1**: Distance to nearest point -- smooth cell interiors
- **F2**: Distance to second nearest -- larger cell patterns
- **F2 - F1**: Cell edge detection -- highlights boundaries between cells

Character: Cell structures, cobblestone, biological tissue, cracked earth.

### Cellular Noise (simplified)

A lightweight version of Voronoi used in texture generation:

```
For each grid cell in 3x3 neighborhood:
    compute distance to cell's random point
    track first and second nearest
result = (second_dist - first_dist) * 2.0 - 1.0
```

This F2-F1 output naturally highlights cell boundaries.

## Color Mapping

Noise values (typically -1.0 to 1.0) are mapped to colors using ramp functions:

### Grayscale
Direct mapping: `value * 0.5 + 0.5` scaled to 0-255.

### Terrain Ramp
Multi-stop gradient simulating natural elevation bands:
- 0.0-0.3: Deep water (dark blue to medium blue)
- 0.3-0.4: Shallow water to sand
- 0.4-0.5: Sand to grass
- 0.5-0.7: Grass to forest
- 0.7-0.85: Forest to mountain rock
- 0.85-1.0: Rock to snow

### Heat Map
Four-stop gradient from cool to hot:
- 0.0-0.25: Dark blue to blue
- 0.25-0.5: Blue to green
- 0.5-0.75: Green to yellow
- 0.75-1.0: Yellow to red

## Texture Application

When noise is applied to textures, it modulates the base color:

```
variation = noise_value * color_variation
final_color = base_color + variation
```

For two-tone materials with a secondary color:
```
blend = clamp(noise_value * 0.5 + 0.5, 0.0, 1.0)
final_color = lerp(base_color, secondary_color, blend) + variation
```

## Speckle Overlay

A separate high-frequency noise pass adds random dot patterns:

```
hash = perlin(x * 1.731, y * 2.399)  // irrational multipliers avoid alignment
if hash > 1.0 - speckle_density * 2.0:
    color = speckle_color
```

Used for sand grains, mineral flecks, and surface imperfections.

## Deterministic Hashing

For features requiring per-cell randomness (Voronoi point placement, brick pattern variation), a fast integer hash provides deterministic pseudo-random values:

```
h = (x * 374761393 + y * 668265263 + seed * 1274126177)
h = (h ^ (h >> 13)) * 1103515245
h = h ^ (h >> 16)
result = (h & 0x7FFFFFFF) / 0x7FFFFFFF  // normalize to [0, 1)
```

## Seeds

Every noise source accepts a `seed` parameter. Related but independent noise channels use offset seeds (e.g., `seed`, `seed + 137`, `seed + 293`) to ensure they produce different patterns while remaining reproducible.
