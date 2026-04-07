# Decal Editor

Create and edit 2D overlays (decals) that are applied on top of 3D object surfaces, layered above the base [surface](surface-editor.md).

## Overview

Decals are 2D designs composed from surface-oriented SDF primitives using boolean operations. They are projected onto 3D object surfaces and rendered on top of the base surface, allowing details like insignias, damage marks, panel lines, racing stripes, and decorative elements without modifying the underlying surface definition.

The term "decal" is used here to describe any surface overlay -- logos, scratches, grime patches, panel details, warning labels, stripes, or decorative motifs.

## Primitives

Decal primitives are designed for mark-making on surfaces rather than general-purpose geometry construction. Each returns a signed distance: negative inside, positive outside, zero on the boundary.

### Compact Primitives

These work well with all projection modes, including triplanar wrapping around edges:

| Primitive | Parameters                               | Description                                                                                                                                 |
| --------- | ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `Spot`    | `radius, falloff`                        | Circle with soft or hard edge. `falloff: 0.0` = hard disc, `1.0` = fully feathered. The basic building block for dots, splatter, and grime. |
| `Ring`    | `radius, thickness`                      | Circle outline. Useful for target marks, rivets, and decorative borders.                                                                    |
| `Arc`     | `radius, thickness, angle, sweep`        | Partial ring. Defined by start `angle` and `sweep` in degrees. For gauges, partial borders, crescent shapes.                                |
| `Polygon` | `radius, sides`                          | Regular N-sided polygon (3 = triangle, 6 = hexagon, etc.).                                                                                  |
| `Star`    | `outer_radius, inner_radius, points`     | N-pointed star.                                                                                                                             |
| `Box`     | `half_width, half_height, corner_radius` | Rectangle with optional rounded corners.                                                                                                    |

### Stroke Primitives

For lines and curves. These are inherently extended shapes -- they can span large areas and cross face boundaries. See [Projection Pairing](#projection-pairing) for guidance on which projection mode to use.

| Primitive     | Parameters                                         | Description                                                                                                                                                                 |
| ------------- | -------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Line`        | `start, end, width`                                | Straight line segment between two points with uniform width.                                                                                                                |
| `Bezier`      | `start, ctrl1, ctrl2, end, width_start, width_end` | Cubic Bezier curve with variable width. The four control points define the curve shape; width tapers linearly from `width_start` to `width_end`.                            |
| `BezierChain` | `points, widths`                                   | Chain of cubic Bezier segments sharing endpoints. `points` has `3n + 1` entries (start, then groups of ctrl1, ctrl2, end). `widths` has `n + 1` entries (one per junction). |

### Bezier Curve Evaluation

The Bezier SDF is computed by finding the closest point on the cubic curve to each evaluation point:

1. The cubic `B(t) = (1-t)^3 * P0 + 3(1-t)^2*t * P1 + 3(1-t)*t^2 * P2 + t^3 * P3` is parameterized by `t` in [0, 1]
2. For each pixel, solve for the `t` that minimizes distance to the curve (iterative Newton's method or analytical root finding on the quintic)
3. The unsigned distance to the curve centerline is `length(pixel - B(t_closest))`
4. The stroke width at that point is `lerp(width_start, width_end, t_closest)`
5. The signed distance is `centerline_distance - width_at_t / 2`

This gives smooth, resolution-independent curved strokes with:
- Precise anti-aliasing from the SDF representation
- Variable width along the stroke length (tapered, flared, or uniform)
- Clean end caps
- Chainable segments that share endpoints for continuous paths

### Example: Curved Racing Stripe

```ron
(primitive: Bezier,
 start: (-0.3, 0.0), ctrl1: (-0.1, 0.15), ctrl2: (0.1, -0.15), end: (0.3, 0.0),
 width_start: 0.03, width_end: 0.02)
```

### Example: S-Curve Panel Line

```ron
(primitive: BezierChain,
 points: [(-0.3, 0.1), (-0.15, 0.2), (0.0, 0.0), (0.0, 0.0),
          (0.0, 0.0), (0.15, -0.2), (0.3, -0.1)],
 widths: [0.01, 0.015, 0.01])
```

## Boolean Operations

Primitives are combined using CSG-style boolean operations:

| Operation            | Formula               | Description                      |
| -------------------- | --------------------- | -------------------------------- |
| `Union`              | `min(a, b)`           | Merge shapes together            |
| `Intersection`       | `max(a, b)`           | Keep only the overlapping region |
| `Subtraction`        | `max(a, -b)`          | Cut shape B from shape A         |
| `SmoothUnion`        | Polynomial smooth min | Merge with rounded blend         |
| `SmoothSubtraction`  | Polynomial smooth max | Cut with rounded blend           |
| `SmoothIntersection` | Polynomial smooth max | Intersect with rounded blend     |

Smooth operations take a `k` parameter controlling the blend radius (typically 0.001-0.2).

### Smooth Blending

The smooth union formula creates organic transitions between shapes:

```
h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0)
result = b + (a - b) * h - k * h * (1.0 - h)
```

This is particularly useful for creating organic decal shapes like splatter patterns, worn edges, and biological motifs.

## Shape Properties

Each shape in a decal composition has:

| Property    | Type            | Default  | Description                                                 |
| ----------- | --------------- | -------- | ----------------------------------------------------------- |
| `primitive` | `PrimitiveType` | required | Which SDF primitive                                         |
| `x, y`      | `f64`           | `0.0`    | Position in normalized coordinates (for compact primitives) |
| `rotation`  | `f64`           | `0.0`    | Rotation around shape center in degrees (math convention)   |
| `op`        | `BoolOp`        | `Union`  | How this shape combines with previous shapes                |
| `smooth_k`  | `f64`           | `0.05`   | Blend radius for smooth operations                          |

Size parameters vary by primitive (see tables above). Stroke primitives use explicit point coordinates instead of `x, y` positioning.

## Visualization Modes

The editor provides three visualization modes:

| Mode               | Description                                                              |
| ------------------ | ------------------------------------------------------------------------ |
| **Solid**          | Anti-aliased filled shape with configurable foreground/background colors |
| **Distance Field** | Raw signed distance visualized as blue (inside) / red (outside)          |
| **Contours**       | Iso-distance contour lines with bright boundary highlight                |

## Composing Decals

Decals are built by layering shapes with boolean operations. The first shape in the list is the base; each subsequent shape is combined with the running result using its `op` field.

### Example: Shield Emblem

A compact decal using rings and boxes -- works well with triplanar projection:

```ron
(
    name: "shield",
    shapes: [
        (primitive: Ring, x: 0.0, y: 0.0, radius: 0.18, thickness: 0.025),
        (primitive: Box, x: 0.0, y: 0.0, half_width: 0.12, half_height: 0.02,
         op: Union),
        (primitive: Box, x: 0.0, y: 0.0, half_width: 0.02, half_height: 0.12,
         op: Union),
    ],
    color: (0.8, 0.5, 0.1),
)
```

### Example: Organic Splatter

Soft spots with smooth blending:

```ron
(
    name: "splatter",
    shapes: [
        (primitive: Spot, x: -0.05, y: 0.02, radius: 0.12, falloff: 0.3),
        (primitive: Spot, x: 0.08, y: -0.03, radius: 0.09, falloff: 0.4,
         op: SmoothUnion, smooth_k: 0.08),
        (primitive: Spot, x: -0.02, y: 0.1, radius: 0.06, falloff: 0.2,
         op: SmoothUnion, smooth_k: 0.06),
    ],
    color: (0.3, 0.15, 0.1),
)
```

### Example: Racing Stripe with Curved Ends

An extended decal using Bezier strokes -- best paired with cylindrical or planar projection:

```ron
(
    name: "racing_stripe",
    shapes: [
        (primitive: BezierChain,
         points: [(-0.4, 0.0), (-0.3, 0.05), (-0.1, 0.05), (0.0, 0.0),
                  (0.0, 0.0), (0.1, -0.05), (0.3, -0.05), (0.4, 0.0)],
         widths: [0.025, 0.03, 0.025]),
    ],
    color: (0.9, 0.9, 0.95),
)
```

## Application to 3D Objects

Decals are not limited to flat surfaces. They wrap around edges, conform to spheres, and cover arbitrary 3D geometry using triplanar projection -- the same world-space evaluation strategy used by [surfaces](surface-editor.md).

### Triplanar Projection

Rather than projecting the decal along a single axis (which stretches at glancing angles and fails at edges), the SDF is evaluated from three orthogonal projections simultaneously and blended based on the surface normal:

```
// In the fragment shader, for each surface point:
normal = abs(world_normal)
weight = normalize(pow(normal, sharpness))  // sharpness controls blend tightness

d_x = evaluate_sdf(world_pos.yz - decal_center.yz)  // project along X
d_y = evaluate_sdf(world_pos.xz - decal_center.xz)  // project along Y
d_z = evaluate_sdf(world_pos.xy - decal_center.xy)  // project along Z

distance = d_x * weight.x + d_y * weight.y + d_z * weight.z
```

The `sharpness` exponent (typically 4.0-8.0) controls how quickly the blend transitions between projections. Higher values produce sharper transitions at edges; lower values produce smoother wrapping.

### How It Works on Each Object Shape

| Object Shape        | Behavior                                                                                                                                                                                       |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Box**             | Decal wraps cleanly across edges and around corners. Each face uses its dominant projection axis, with smooth blending at the bevels.                                                          |
| **Sphere**          | Smooth coverage over the entire surface. The three projections blend continuously based on the surface normal direction. Slight distortion at the poles where two projections compete equally. |
| **Cylinder**        | Wraps around the barrel via X/Z projections, blends onto the caps via Y projection. Clean transition at the rim.                                                                               |
| **Compound shapes** | Works on any geometry. The projection is purely a function of world position and surface normal -- it does not depend on mesh topology, UV mapping, or shape type.                             |

### Projection Modes

| Mode                | Description                                                                                                                                |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `Triplanar`         | Default. Wraps around all geometry using normal-weighted blending. Best for compact decals.                                                |
| `Planar(axis)`      | Single-axis projection. Decal only appears on surfaces facing the projection axis. Good for flat faces or restricting a decal to one side. |
| `Cylindrical(axis)` | Projects radially around an axis. Ideal for stripes, bands, and labels that wrap around cylindrical or elongated parts.                    |

### Projection Pairing

The choice of projection mode depends on the decal's shape:

| Decal Type                                    | Recommended Projection    | Why                                                                                                                                             |
| --------------------------------------------- | ------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| Compact shapes (spots, rings, emblems, stars) | `Triplanar`               | Small relative to the object. Wraps naturally around edges with no artifacts.                                                                   |
| Straight stripes, panel lines                 | `Planar` or `Cylindrical` | Extended along one direction. Triplanar would evaluate the stripe on all three axis planes, producing ghost copies on perpendicular faces.      |
| Curved strokes wrapping around a part         | `Cylindrical`             | The stroke follows the part's circumference. Cylindrical projection maps the decal's X axis around the barrel, so the curve wraps continuously. |
| Large insignia spanning a flat panel          | `Planar`                  | Covers one face only. No need for wrapping, and planar avoids blend-zone artifacts at edges.                                                    |
| Grime, weathering, damage splatter            | `Triplanar`               | Organic, non-directional. Triplanar blending at edges actually helps -- weathering should look natural at seams.                                |

### Decal Placement

Each decal instance on a 3D object has:

| Property          | Type              | Default           | Description                                                |
| ----------------- | ----------------- | ----------------- | ---------------------------------------------------------- |
| `decal`           | `String`          | required          | Name of the decal definition to apply                      |
| `center`          | `(f32, f32, f32)` | `(0.0, 0.0, 0.0)` | World-space center of the decal projection                 |
| `scale`           | `f32`             | `1.0`             | Size of the decal in world units                           |
| `rotation`        | `f32`             | `0.0`             | Rotation of the decal pattern in degrees (math convention) |
| `projection`      | `ProjectionMode`  | `Triplanar`       | `Triplanar`, `Planar(axis)`, or `Cylindrical(axis)`        |
| `blend_sharpness` | `f32`             | `6.0`             | Triplanar blend exponent. Only used with `Triplanar`.      |
| `opacity`         | `f32`             | `1.0`             | Overall decal opacity (0.0-1.0)                            |

### Compositing

Decals are alpha-blended over the base surface. The alpha value is derived from the SDF distance for anti-aliased edges:

```
coverage = 1.0 - smoothstep(-pixel_size, pixel_size, sdf_distance)
final_alpha = coverage * decal_opacity
final_color = decal_color * final_alpha + surface_color * (1.0 - final_alpha)
```

Inside the shape (`sdf_distance < 0`), coverage approaches 1.0. Outside, it approaches 0.0. The `smoothstep` transition spans exactly one pixel at the boundary for clean anti-aliasing.

### Layering

Multiple decals can be layered on a single object. Each decal is composited in order over the result of the previous:

1. Base surface is rendered
2. Decal 1 is alpha-blended over
3. Decal 2 is alpha-blended over the result
4. And so on

Each decal has independent position, scale, rotation, projection mode, and opacity. Later decals occlude earlier ones where they overlap.

## Interactive Editing

The egui side panel provides:
- **Shape list** with add/remove controls
- **Primitive type selector** for each shape
- **Position controls**: sliders for compact primitives, control point editors for strokes
- **Size and rotation** controls that adapt to the selected primitive
- **Bezier handle editor**: drag control points directly in the viewport for Bezier strokes
- **Width taper** controls for stroke primitives
- **Boolean operation selector** with smooth blend radius
- **Visualization mode** toggle (Solid / Distance Field / Contours)
- **Grid overlay** toggle for alignment
- **3D preview** toggle to see the decal projected onto a test shape (cube, sphere, cylinder)

Compact shapes can be repositioned by left-click dragging in the viewport. Bezier control points are dragged individually.

## Command Line

```bash
# Interactive decal editor
cargo run -- decal

# Load a decal definition
cargo run -- decal data/decals/shield_emblem.ron

# Preview on a 3D shape
cargo run -- decal data/decals/racing_stripe.ron --preview cylinder
```
