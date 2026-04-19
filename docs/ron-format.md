# RON Format

Shapes in this codebase are defined in RON (Rusty Object Notation), a human-readable data format native to the Rust ecosystem. RON files are the single source of truth — every shape can be fully described in text.

## Why RON

- **Version control**: RON files diff cleanly in git, unlike binary assets
- **Procedural generation**: Scripts and tools can generate RON programmatically
- **Human-readable**: Edit shapes in any text editor
- **Type-safe**: RON maps directly to Rust structs via serde deserialization
- **Composable**: Imports allow reuse across files

## Syntax Overview

RON is similar to Rust literal syntax:

```ron
(
    // Structs use parentheses
    name: "scout_bot",

    // Tuples
    bounds: (-2, -1, -2, 2, 1, 2),

    // Enums (no parameters)
    corner: (MinX, MinY, MinZ),

    // Lists
    children: [
        (name: "part_a"),
        (name: "part_b"),
    ],
)
```

## Shape Files (`.shape.ron`)

A `.shape.ron` file is a `Vec<SpecNode>` — a flat array of parts. Each part may have nested `children`.

```ron
[
    (
        name: "chassis",
        bounds: (-2, 0, -1, 2, 1, 1),
        tags: ["green3"],
        children: [
            (
                name: "head",
                bounds: (-1, 1, -1, 1, 2, 1),
                tags: ["red2"],
            ),
        ],
    ),
    (
        name: "wheel",
        import: "wheel",
        bounds: (1, 0, -2, 2, 1, -1),
        symmetry: [MirrorX, MirrorZ],
    ),
]
```

Key features:
- **Bounds**: `(x1, y1, z1, x2, y2, z2)` — two integer corners of the part's bounding box.
- **Primitives**: A bounded node with no `corner`/`clip`/`faces` is a `Box`. Add `corner: (Face, Face, Face)` for a corner primitive, `clip: (Face, Face, Face)` for an inverse corner, or `faces: (Face, Face)` for a wedge. The face tuple specifies which sides of the bounding box the primitive's filled vertex/edge sits against.
- **Tags**: Per-node `tags: ["..."]` array for shared appearance. (Tag → texture binding will be wired up when the surface tool exists; today tags are recorded but not consumed visually.)
- **Imports**: Reference another `.shape.ron` file by name; the imported shape is remapped to fit the placement bounds via integer multiplication.
- **Symmetry**: `symmetry: [MirrorX, Rotate90_XZ, ...]` takes the closure of the listed operations and deduplicates by canonical bounds + CSG signature. Common usage: `[MirrorX]` = 2 copies, `[MirrorX, MirrorY, MirrorZ]` = 8.
- **Subtract**: `subtract: true` removes the part's volume from sibling/parent geometry instead of adding it.
- **Animations**: Named states with per-part motion channels (can be on any node).

See [Object Editor](object-editor.md) for the complete node property reference and [Composition Model](composition-model.md) for the design rationale.

## Angle Convention

Angle values follow math convention: 0° = right (+X), counter-clockwise positive. Code converts to radians at the deserialization boundary. Currently only animation channels use angle values.

## Deserialization

```rust
let ron_str = std::fs::read_to_string(path)?;
let parts: Vec<SpecNode> = ron::de::from_str(&ron_str)?;
```

Fields use `#[serde(default)]` extensively, so you only specify what differs from defaults.
