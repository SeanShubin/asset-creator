# Runtime Collision Detection (Future)

Design notes for runtime collision queries against rendered shape entities. **Not yet implemented.** This document captures the architecture so it can be picked up later without rederiving it.

## Context

The shape pipeline produces a hierarchical entity tree at spawn time:

```
ShapeRoot entity
├── ShapePart "chassis"   (Transform, fused mesh of chassis cells)
├── ShapePart "arm_left"  (Transform)
│   ├── ShapePart "hand"  (Transform, fused mesh of hand cells)
│   └── (fused mesh of arm cells excluding hand)
└── ...
```

Each part contains a single fused mesh (or two: one non-emissive, one emissive). The fused mesh is built from individual cells during compile, but the cell positions are discarded once fusion completes — only vertices survive.

For game-time collision queries, the cell positions are exactly what we need. A projectile passing through a "near miss" region inside the chassis's AABB but outside any cell should report no hit. AABB-only collision can't tell that apart from a real hit.

## The query model

Collision queries against a rendered entity run in **three stages**, each cheaper than the previous and rejecting most queries before reaching the next stage:

### Stage 1 — Whole-entity AABB

A single AABB-vs-query test against the union of all part AABBs in world space. Bevy already maintains this via `bevy::render::primitives::Aabb` on the root entity. The vast majority of queries fail at this stage, since most queries don't go anywhere near most entities.

### Stage 2 — Per-part AABB

If stage 1 passes, walk the entity's child `ShapePart` entities and AABB-test each one. Most parts will reject. The query continues into the few parts whose AABBs the query overlaps.

For typical entities with a handful of parts, this is brute-force iteration — no spatial index needed. For entities with dozens of parts, a per-entity bounding volume hierarchy could help, but profile first; the part counts here are small.

### Stage 3 — Per-cell

For each part that survived stage 2, look up the part's **cell occupancy index** (a `HashSet<(i32, i32, i32)>` of integer cell positions in part-local coordinates) and test the query against it. This is the only stage that distinguishes "near miss inside the AABB" from "actual hit."

The query type determines the test:

| Query type        | Stage-3 test                                                                                                                             |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| **Point**         | Floor world position to integer cell, check `cells.contains(&cell)`. O(1)                                                                |
| **Ray**           | DDA voxel traversal from ray origin to ray end, check each cell along the line. O(cells along ray)                                       |
| **AABB / hitbox** | Iterate either the cells inside the query AABB or the part's cells, whichever is smaller; check overlap. O(min(query cells, part cells)) |

For the shape sizes the project produces (parts in the dozens, cells in the hundreds), all three are sub-microsecond per query.

## Data structure

The cell index lives as a Bevy component on each `ShapePart` entity:

```rust
#[derive(Component)]
pub struct PartCells {
    /// Integer cells (in part-local coordinates) occupied by this part's
    /// primitives. Used as the final-stage hit test inside the part's AABB.
    pub cells: HashSet<(i32, i32, i32)>,
}
```

Optionally, if the game needs per-cell metadata (color, name, primitive type, surface material), the value type can be richer:

```rust
pub struct PartCells {
    pub cells: HashMap<(i32, i32, i32), CellInfo>,
}

pub struct CellInfo {
    pub shape: PrimitiveShape,
    pub color: Color3,
    // ...whatever the game needs per cell
}
```

The map is populated during `render::compile`'s fusion step. The fusion code already iterates surviving cells per primitive — it just needs to additionally write each cell to the `PartCells` map and attach the resulting component to the part entity at spawn time.

## Coordinate space

**Part-local cell coordinates.** When the entity transforms (animation, repositioning, parent transforms), the cells move with the part — no rebuild needed. Query code transforms the world-space query into part-local space using the inverse of the part's `GlobalTransform`, then floors to cell coordinates.

The alternative, shape-root-local coordinates, would skip the per-part inverse but break for animated rigs where parts rotate independently. Part-local is the right choice for any rig that animates.

## Sample query function

Sketch (will need refinement against real Bevy query patterns):

```rust
pub fn point_hit(
    world_pos: Vec3,
    entity_root: Entity,
    aabb_query: &Query<(&GlobalTransform, &Aabb)>,
    part_query: &Query<(Entity, &ShapePart, &GlobalTransform, Option<&PartCells>)>,
) -> Option<HitInfo> {
    // Stage 1: whole-entity AABB
    let (root_tf, root_aabb) = aabb_query.get(entity_root).ok()?;
    if !aabb_contains_point(root_tf, root_aabb, world_pos) {
        return None;
    }

    // Stage 2 + 3: walk parts, AABB filter, then cell lookup
    for (part_entity, part_name, part_tf, cells) in part_query.iter() {
        let (part_aabb_tf, part_aabb) = aabb_query.get(part_entity).ok()?;
        if !aabb_contains_point(part_aabb_tf, part_aabb, world_pos) {
            continue;
        }
        let Some(cells) = cells else { continue };

        let local = part_tf.compute_matrix().inverse().transform_point3(world_pos);
        let cell = (
            local.x.floor() as i32,
            local.y.floor() as i32,
            local.z.floor() as i32,
        );
        if cells.cells.contains(&cell) {
            return Some(HitInfo {
                entity: part_entity,
                part_name: part_name.name.clone(),
                cell,
            });
        }
    }
    None
}
```

## Implementation cost when picked up

Roughly:

- **`render::compile`**: during fusion, also accumulate cells into a `HashMap<(i32, i32, i32), CellInfo>` per `CompiledShape`. ~30 lines.
- **`CompiledShape`**: add a `cells` field to carry the map up to the interpreter. ~5 lines.
- **`interpreter::attach_compiled`**: when spawning a `ShapePart` entity, attach the `PartCells` component if the map is non-empty. ~10 lines.
- **New module `runtime_collision.rs`** with point/ray/AABB query functions. ~150 lines including tests.
- **Tests**: point query against a known shape, ray query through a hollow region, AABB query overlap. ~50 lines.

Total: roughly 250 lines added, no deletions.

## What this does NOT need

- A general spatial index (BVH, kd-tree, octree). The shape hierarchy already partitions space well enough for typical entity counts.
- Sub-cell precision. Cells are the atomic unit of geometry; "I hit cell (3, 4, 5)" is the level of precision the gameplay layer cares about.
- Caching across frames. Each query is independent; the `PartCells` map IS the cache.
- Greedy meshing or other geometric optimization. The HashSet lookup is faster than any geometric refinement could be at these sizes.
- Updates when parts animate. Cells live in part-local coordinates and are static for the life of the spawned entity.

## Relationship to spec-time collision check

The spec-time check (`collect_occupancy` in `spec.rs`) and the runtime check are **different uses of the same data**. The cells a part claims at compile time are exactly the cells the game wants to query at runtime.

The two checks differ in:

|                      | Spec-time                          | Runtime                                                  |
| -------------------- | ---------------------------------- | -------------------------------------------------------- |
| **Purpose**          | Authoring correctness              | Gameplay                                                 |
| **When**             | Shape load / reload                | Every projectile / hitbox / ray                          |
| **Scope**            | One shape in isolation             | One entity instance per query                            |
| **Coordinate space** | Shape-native cell coords           | Part-local cell coords                                   |
| **Output**           | Collision count + list of overlaps | Yes/no + cell + part identity                            |
| **Imports**          | Opaque (claim placement AABB)      | Opaque (the imported shape's parts have their own cells) |

The runtime side benefits from the spec-time check having already verified the shape: a shape with no spec-time collisions has no overlapping cells anywhere, so the runtime query for any single cell can never return more than one hit per entity per cell.

## Sequencing notes

Implement this only when the game side actually needs collision queries. Doing it speculatively risks the API not matching the actual call sites the gameplay code wants. The architecture documented here is stable enough to pick up on demand without further design work — the shape pipeline already produces all the data needed.
