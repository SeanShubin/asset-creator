# RON Format

All assets in the application are defined in RON (Rusty Object Notation), a human-readable data format native to the Rust ecosystem. RON files are the single source of truth for procedural generation -- every shape, texture, tileset, decal, and world can be fully described in text.

## Why RON

- **Version control**: RON files diff cleanly in git, unlike binary assets
- **Procedural generation**: Scripts and tools can generate RON programmatically
- **Human-readable**: Edit assets in any text editor
- **Type-safe**: RON maps directly to Rust structs via serde deserialization
- **Composable**: Templates and references allow reuse across files

## Syntax Overview

RON is similar to Rust literal syntax:

```ron
(
    // Structs use parentheses
    name: "scout_bot",
    health: 100,

    // Tuples
    position: (1.0, 2.0, 3.0),

    // Enums
    shape: Box(size: (0.5, 0.3, 0.4)),

    // Options
    color: Some((0.8, 0.2, 0.1)),
    secondary: None,

    // Maps
    templates: {
        "wheel": ( /* ... */ ),
        "arm": ( /* ... */ ),
    },

    // Lists
    children: [
        (name: "part_a"),
        (name: "part_b"),
    ],
)
```

## Asset File Types

| Extension | Editor | Root Type | Description |
|-----------|--------|-----------|-------------|
| `.surface.ron` | Surface Editor | `SurfaceDef` | Visual appearance definition (color, noise, pattern, roughness) |
| `.shape.ron` | Object Editor | `ShapeFile` | 3D object with templates and animations |
| `.tileset.ron` | Tileset Editor | `TilesetDef` | 47-blob tileset referencing face/edge surfaces |
| `.decal.ron` | Decal Editor | `DecalDef` | SDF shape composition for surface overlays |
| `.world.ron` | World Editor | `WorldDef` | Biome terrain generation parameters |

## Shape Files (`.shape.ron`)

The most complex format, supporting hierarchical composition:

```ron
(
    templates: {
        "wheel": (
            shape: Cylinder(radius: 0.18, height: 0.1),
            color: (0.25, 0.25, 0.28),
            orient: Z,
        ),
    },
    root: (
        name: "vehicle",
        children: [
            (
                name: "chassis",
                shape: Box(size: (1.4, 0.5, 0.8)),
                at: (0.0, 0.5, 0.0),
                surface: "rusted_steel",
                decals: [
                    (decal: "insignia", center: (0.0, 0.0, 0.41), scale: 0.3),
                ],
                children: [
                    (template: "wheel", at: (0.5, -0.3, 0.3), mirror: X),
                    (template: "wheel", at: (0.5, -0.3, -0.3), mirror: X),
                ],
            ),
        ],
    ),
    animations: [
        (
            name: "drive",
            channels: [
                (part: "wheel", property: Rotation, axis: Z,
                 motion: Spin(rate: 8.0)),
            ],
        ),
    ],
)
```

Key features:
- **Templates**: Reusable subtrees referenced by name
- **Surfaces**: Named [surface](surface-editor.md) references for visual appearance
- **Decals**: [Decal](decal-editor.md) instances placed on geometry via triplanar projection
- **Mirror**: `mirror: X` duplicates a subtree reflected across an axis
- **Repeat**: `repeat: (count: 5, spacing: 0.15, along: Z)` for linear arrays
- **Animations**: Named states with per-part motion channels

See [Object Editor](object-editor.md) for the complete node property reference.

## Surface Files (`.surface.ron`)

```ron
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

Surfaces define visual appearance independently of output format. The same surface renders as CPU pixels (tilesets, exports) or GPU shaders (3D objects). See [Surface Editor](surface-editor.md) for parameter details.

## Tileset Files (`.tileset.ron`)

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

Face and edge zones reference surfaces by name or define them inline. See [Tileset Editor](tileset-editor.md) for the complete reference.

## Decal Files (`.decal.ron`)

Defines the decal design (SDF shape composition):

```ron
(
    name: "insignia",
    shapes: [
        (primitive: Circle, x: 0.0, y: 0.0, size_a: 0.2),
        (primitive: Circle, x: 0.0, y: 0.0, size_a: 0.17,
         op: Subtraction),
        (primitive: Box, x: 0.0, y: 0.0, size_a: 0.15, size_b: 0.03,
         op: Union),
    ],
    color: (0.8, 0.5, 0.1),
)
```

Decals are placed on 3D objects as instances with position, scale, and projection mode:

```ron
decals: [
    (decal: "insignia", center: (0.0, 0.0, 0.41), scale: 0.3),
    (decal: "scratch", center: (0.2, 0.1, 0.0), scale: 0.5,
     projection: Planar(Z), opacity: 0.6),
]
```

Decals use triplanar projection by default, wrapping around edges, spheres, and arbitrary geometry. See [Decal Editor](decal-editor.md) for the primitive, operation, and projection reference.

## World Files (`.world.ron`)

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
)
```

See [World Editor](world-editor.md) for the complete parameter reference.

## Deserialization

All RON files are deserialized using `serde` with `ron::de::from_str`:

```rust
let ron_str = std::fs::read_to_string(path)?;
let shape_file: ShapeFile = ron::de::from_str(&ron_str)?;
```

Fields use `#[serde(default)]` extensively so that most properties are optional -- you only need to specify what differs from defaults.
