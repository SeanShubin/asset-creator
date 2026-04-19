# Object Editor

Live edit and preview 3D objects defined in RON format, with support for animations, hierarchical part trees, imports, and symmetry.

## Overview

The object editor loads `.shape.ron` files and renders each as a tree of Bevy entities with meshes. The shape list, camera controls, animation state, and part tree all live in the left side panel; the central viewport renders the selected shape. External edits to the file hot-reload within ~500ms.

## Shape Definition Format

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

## Primitives

A bounded node's primitive is inferred from the presence (or absence) of `corner`, `clip`, or `faces`:

| Primitive       | How to specify                             | Geometry                                            |
| --------------- | ------------------------------------------ | --------------------------------------------------- |
| `Box`           | (no `corner`/`clip`/`faces` field)         | Fills the entire bounds                             |
| `Wedge`         | `faces: (Face, Face)` — two adjacent faces | Triangular prism; the two faces are the filled half |
| `Corner`        | `corner: (Face, Face, Face)` — three faces | Tetrahedron with vertex at the meeting corner       |
| `InverseCorner` | `clip: (Face, Face, Face)` — three faces   | Box minus one corner (complement of `Corner`)       |

Faces are `MinX`, `MaxX`, `MinY`, `MaxY`, `MinZ`, `MaxZ`. For Wedge, the two faces share an edge that becomes the slope. For Corner / InverseCorner, the three faces meet at the relevant vertex.

## Node Properties

| Property     | Type                | Description                                                                                                  |
| ------------ | ------------------- | ------------------------------------------------------------------------------------------------------------ |
| `name`       | `Option<String>`    | Identifier used for animation targeting and UI display                                                       |
| `bounds`     | `Option<Bounds>`    | Two integer corners `(x1, y1, z1, x2, y2, z2)`. Required for any node that contributes geometry.             |
| `faces`      | `Option<[Face; 2]>` | Specifies a Wedge primitive (see above)                                                                      |
| `corner`     | `Option<[Face; 3]>` | Specifies a Corner primitive (see above)                                                                     |
| `clip`       | `Option<[Face; 3]>` | Specifies an InverseCorner primitive (see above)                                                             |
| `tags`       | `Vec<String>`       | Free-form tags for shared appearance. Recorded but not yet consumed visually.                                |
| `import`     | `Option<String>`    | Name of another `.shape.ron` file to import. Imported shape is remapped to fit `bounds` via integer scaling. |
| `children`   | `Vec<SpecNode>`     | Child nodes in the hierarchy                                                                                 |
| `rotate`     | `Vec<SymOp>`        | Pre-symmetry orientation transforms, composed left-to-right                                                  |
| `symmetry`   | `Vec<SymOp>`        | Symmetry generators. The system takes the closure and deduplicates by canonical bounds + CSG signature.      |
| `subtract`   | `bool`              | If true, this part's volume is removed from sibling/parent geometry instead of added                         |
| `animations` | `Vec<AnimState>`    | Named animation states with per-part motion channels (can be on any node)                                    |

`SymOp` values: `MirrorX`, `MirrorY`, `MirrorZ`, `Rotate90_XY`, `Rotate90_XZ`, `Rotate90_YZ`, `Rotate180_XY`, `Rotate180_XZ`, `Rotate180_YZ`. All operations are signed axis permutations and preserve integer coordinates.

## Imports

Imports reference another `.shape.ron` file by name. The imported shape's native AABB is remapped onto the placement `bounds` using only integer multiplication — no division or rounding, so coordinates stay exact.

```ron
[
    (name: "front_wheel", import: "wheel",
     bounds: (1, 0, -2, 2, 1, -1),
     symmetry: [MirrorX]),
    (name: "rear_wheel", import: "wheel",
     bounds: (-2, 0, -2, -1, 1, -1),
     symmetry: [MirrorX]),
]
```

If `bounds` is omitted on an import, the registry resolves the imported shape's own AABB and uses that.

## Symmetry

The `symmetry` field applies signed-axis permutations to the node's bounds and inferred primitive, takes the closure under composition, and deduplicates copies whose canonical bounds + CSG signature match. Common patterns:

| Generators                                | Resulting copies | Use case             |
| ----------------------------------------- | ---------------- | -------------------- |
| `[MirrorX]`                               | 2                | Bilateral symmetry   |
| `[MirrorX, MirrorZ]`                      | 4                | Quadrant symmetry    |
| `[MirrorX, MirrorY, MirrorZ]`             | 8                | Octant symmetry      |
| `[Rotate90_XZ]`                           | 4                | 4-fold rotational    |
| `[Rotate90_XY, Rotate90_XZ, Rotate90_YZ]` | 24               | Full cube rotational |

Deduplication means a centered Box with `[MirrorX]` produces 1 copy, not 2 — the mirrored copy is identical to the original.

## Animation System

Animations are named states. Each state contains channels that target parts by name. Any node can carry an `animations` field — it is not limited to the root.

### Motion Types

| Motion      | Parameters                 | Description                                     |
| ----------- | -------------------------- | ----------------------------------------------- |
| `Oscillate` | `amplitude, speed, offset` | `sin(phase * speed + offset) * amplitude`       |
| `Spin`      | `rate`                     | Continuous rotation: `phase * rate`             |
| `Bob`       | `amplitude, freq`          | Time-based sine bob (independent of walk phase) |

### Animation Properties

| Property      | Description                                |
| ------------- | ------------------------------------------ |
| `Rotation`    | Rotate around the specified axis (radians) |
| `Translation` | Translate along the specified axis         |

### Example

```ron
animations: [
    (
        name: "walk",
        channels: [
            (part: "arm", property: Rotation, axis: X,
             motion: Oscillate(amplitude: 0.25, speed: 10.0, offset: 0.0)),
            (part: "leg", property: Rotation, axis: X,
             motion: Oscillate(amplitude: 0.4, speed: 10.0, offset: 3.14)),
        ],
    ),
]
```

## Left-Panel UI

The left side panel contains, top to bottom:

1. **Shape list.** Click to load. Reloads the editor view with the selected shape.
2. **Camera controls.** Yaw / pitch / zoom drag-values, six fixed-view buttons (Front / Right / Top / Back / Left / Bottom) plus Reset.
3. **Animation controls.** Animation state selector and speed slider (only visible when the loaded shape has animations).
4. **Part tree.** Tri-state visibility toggles per node:
   - `[+]` visible, `[-]` hidden, `[~]` mixed
   - Clicking a node toggles visibility for the entire subtree
   - Subtractive parts render in blue; parts involved in cell collisions render in red
5. **Errors.** Per-file parse errors when any are present.

## Camera Controls

| Input             | Action                    |
| ----------------- | ------------------------- |
| Left mouse drag   | Orbit camera              |
| Middle mouse drag | Pan camera                |
| Scroll wheel      | Zoom (orthographic scale) |
| Arrow keys        | Orbit camera              |
| R                 | Reload the current shape  |
| Tab               | Cycle animation state     |

## Command Line

```bash
# Open the editor with no shape loaded; pick from the list
cargo run

# Open with a specific shape loaded
cargo run -- data/shapes/scout_bot.shape.ron
```
