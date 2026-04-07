# SDF Primitives

Signed distance field shapes and boolean operations. This document covers the core SDF math shared across the application. The [Decal Editor](decal-editor.md) builds on these foundations with its own surface-oriented primitive set (Spot, Ring, Arc, Bezier strokes, etc.).

## Overview

Signed Distance Functions (SDFs) define shapes implicitly: for any point in space, the function returns the shortest distance to the shape's surface. The sign indicates whether the point is inside (negative) or outside (positive). This representation enables:

- Resolution-independent rendering with perfect anti-aliasing
- Boolean composition (union, intersection, subtraction) via simple math
- Smooth blending between shapes
- Distance-based effects (outlines, glows, drop shadows)

## 2D Primitives

### Circle

```
sd_circle(point, center, radius) =
    length(point - center) - radius
```

The simplest SDF. Returns exact Euclidean distance.

### Box (Rectangle)

```
sd_box(point, center, half_width, half_height) =
    let d = abs(point - center) - half_size
    length(max(d, 0)) + min(max(d.x, d.y), 0)
```

The outer distance is Euclidean (rounded corners at distance); the inner distance is Chebyshev (flat sides).

### Line Segment

```
sd_line(point, a, b, thickness) =
    let t = clamp(dot(point-a, b-a) / dot(b-a, b-a), 0, 1)
    length(point - a - (b-a)*t) - thickness
```

Projects the point onto the segment, then measures the perpendicular distance minus thickness.

### Equilateral Triangle

Computed using signed edge distances. For each of the three edges, compute the signed distance from the point to the edge's half-plane. Inside: all three are negative, return the maximum (closest to surface). Outside: return the distance to the nearest edge segment.

### Star

N-pointed star defined by outer radius (tip distance) and inner radius (notch distance):

1. Convert to polar coordinates relative to center
2. Fold the angle into a single sector using `angle % (TAU / n_points)`
3. Compute signed distance to the edge line connecting tip to notch
4. For exterior points, compute distance to the nearest edge segment

Parameters: `outer_radius`, `inner_radius`, `points` (3-12).

## Boolean Operations

Boolean operations combine two SDF values `a` and `b` (where `a` is the accumulated scene and `b` is the new shape):

### Hard Operations

| Operation        | Formula      | Description                      |
| ---------------- | ------------ | -------------------------------- |
| **Union**        | `min(a, b)`  | Merge: inside either shape       |
| **Intersection** | `max(a, b)`  | Overlap only: inside both shapes |
| **Subtraction**  | `max(a, -b)` | Cut: inside A but outside B      |

These produce exact distance fields outside but only approximate distances near the operation boundary.

### Smooth Operations

Smooth variants blend the transition over a radius `k`:

#### Smooth Union

```
h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0)
result = b + (a - b) * h - k * h * (1.0 - h)
```

Creates a fillet/blend between merging shapes. Larger `k` = rounder blend.

#### Smooth Subtraction

```
h = clamp(0.5 - 0.5 * (a + b) / k, 0.0, 1.0)
result = a + (-b - a) * h + k * h * (1.0 - h)
```

Cuts with a rounded transition at the cut boundary.

#### Smooth Intersection

```
h = clamp(0.5 - 0.5 * (b - a) / k, 0.0, 1.0)
result = b + (a - b) * h + k * h * (1.0 - h)
```

Intersects with a rounded chamfer at the intersection edge.

### Choosing k Values

| k Value    | Character                          |
| ---------- | ---------------------------------- |
| 0.001-0.01 | Nearly sharp, barely visible blend |
| 0.02-0.05  | Subtle rounding, mechanical feel   |
| 0.05-0.1   | Visible organic blending           |
| 0.1-0.2    | Very soft, blobby transitions      |

## Rendering

### Anti-Aliased Solid Fill

```
pixel_size = 1.0 / texture_width
coverage = 1.0 - smoothstep(-pixel_size, pixel_size, distance)
color = lerp(background, foreground, coverage)
```

The `smoothstep` function provides a smooth transition across exactly one pixel at the shape boundary, producing clean anti-aliased edges.

### Smoothstep

```
smoothstep(edge0, edge1, x) =
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0)
    t * t * (3.0 - 2.0 * t)
```

### Distance Field Visualization

Color the signed distance directly:
- **Inside** (d < 0): Blue, intensity proportional to `|d|`
- **Outside** (d > 0): Red, intensity proportional to `d`
- **Boundary** (d = 0): Bright white line

### Contour Visualization

Draw iso-distance lines at regular intervals:
```
contour_spacing = 0.02
contour = fract(abs(distance) / contour_spacing)
line = smoothstep(0.4, 0.5, contour) * (1.0 - smoothstep(0.5, 0.6, contour))
boundary = 1.0 - smoothstep(0.0, pixel_size * 2.0, abs(distance))
```

## Rotation

Shapes support rotation by transforming the evaluation point before computing the SDF:

```
dx = point.x - center.x
dy = point.y - center.y
rotated_x = dx * cos(angle) + dy * sin(angle) + center.x
rotated_y = -dx * sin(angle) + dy * cos(angle) + center.y
distance = primitive(rotated_x, rotated_y, ...)
```

Rotation is applied per-shape before the SDF evaluation, so the shape rotates around its own center.

## Scene Evaluation

A complete SDF scene is evaluated by processing shapes in order:

```
distance = shapes[0].evaluate(point)
for shape in shapes[1..]:
    shape_dist = shape.evaluate(point)
    distance = shape.combine(distance, shape_dist)
```

The first shape establishes the base distance field. Each subsequent shape modifies it using its boolean operation. Order matters for non-commutative operations (subtraction).

## RON Representation

```ron
(
    shapes: [
        (
            primitive: Circle,
            x: -0.1,
            y: 0.0,
            size_a: 0.2,
            rotation: 0.0,
        ),
        (
            primitive: Box,
            x: 0.1,
            y: 0.0,
            size_a: 0.15,
            size_b: 0.15,
            op: SmoothUnion,
            smooth_k: 0.05,
        ),
    ],
)
```
