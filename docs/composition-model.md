# Composition Model

This document records the design decision behind how shapes are composed from primitives, and why a bounds-based composition model was chosen over a fixed-cell grid.

## Decision

Shapes are authored as **bounds-based compositions of stretchable primitives**, not as fills of a fixed-cell grid.

A shape is a tree whose leaves are primitives from a small fixed alphabet (currently `Box`, `Wedge`, `Corner`) and whose interior nodes are composites. Every primitive and composite carries an explicit integer **bounding box**. Composition operations are restricted to the integer-rational affine group: per-axis stretch, the 48 axis-aligned rotations and mirrors, and integer translation.

## The "Stretchy Legos" Model

The mental model is a small set of LEGO-like bricks that can be stretched along each axis independently, rotated and mirrored to any of 48 axis-aligned orientations, and snapped together at integer coordinates. Composites can be nested and reused, and the resulting tree is the shape.

### Rational coordinates via integer rescaling

Coordinates are stored as `i32` (or similar integer type), but the model is morally a **rational coordinate system**. Sub-unit precision is achieved not by introducing fractions but by **rescaling the parent unit**: if a sub-assembly needs to place something at "half a unit," the parent's unit is doubled so that "half" becomes "one whole unit at the parent level." The rationality is hidden inside the choice of unit at each composition level, and integer arithmetic does the rest.

This is structurally identical to how a CAD kernel handles exact rational geometry, or how music notation handles tuplets — keep a denominator on the side and let the numerators be integers.

### Why integers, not floats

Because all arithmetic stays in the integer domain, **two faces that should coincide always do, bit for bit**. There is no float drift, no welding tolerance, and no "are these vertices the same?" question at primitive boundaries. This makes the model exact in a way no floating-point format can be.

### Address space

`i32` provides roughly `2.1 × 10⁹` units along each axis. Even with deep nesting and aggressive rescaling, the practical limit is reached far later than display resolution would matter for. `i64` would extend this further and is available if needed.

## Advantages

- **Exact arithmetic.** No floating-point drift. Vertex coincidence at shared faces is bit-exact, not "within tolerance."
- **Continuous (rational) slopes.** Stretching a `Wedge` to bounds `p × q` produces a hypotenuse at `arctan(p/q)`. The set of expressible slopes is dense in `[0°, 90°]`, in contrast to the three discrete angles a fixed-cell grid permits (0°, 45°, ≈54.7°).
- **Compact authoring of symmetric shapes.** Mirror flags and named composites let a shape with N-fold symmetry be written once and replicated declaratively, instead of being repeated N times in a dense grid.
- **Resolution independence.** Any composite can be uniformly rescaled by an integer factor without changing its meaning. There is no "native resolution" baked into the format.
- **Recursive reuse.** Composites are first-class and can be transformed and embedded inside other composites. A leaf is a brick; everything else is a tree.
- **Sparse by default.** Empty space costs nothing. A mostly-empty model has a mostly-empty file.
- **Hand-authorable.** Each entry in a `.shape.ron` file is an intentional declarative statement that a human can read, write, and diff.
- **Watertight by construction.** A shape built from watertight primitives is itself a closed point set. The union of closed solids is closed.

## Limitations

- **No structural prevention of T-junctions.** A T-junction occurs when one primitive's edge endpoint lands in the middle of another primitive's edge. The format permits this; avoiding it is a matter of authoring discipline (or a static adjacency check at load time). See the T-junctions section below.
- **No `O(1)` neighbor queries.** Finding what is adjacent to a given primitive requires scanning the bounds list (or maintaining an index). This is acceptable for static asset authoring but unsuitable for use cases that need per-cell queries every frame (lighting bake, fluid simulation, structural analysis, pathfinding).
- **Not the right substrate for procedural generation from cell-grid noise.** Algorithms that fill cells from a Perlin noise function, cellular automaton, or Wave Function Collapse expect a uniform grid. The stretchy-lego model is the wrong shape for that workflow. If procedural cell-grid generation becomes a goal, a separate fixed-cell representation should be used as input and converted to bounds-based composites for storage.
- **Irrational slopes are not exactly representable.** Angles like `30°` (which is `arctan(1/√3)`) can be approximated to arbitrary precision by raising the denominator, but cannot be hit exactly. For modular and angular assets this is not a real limitation; for free-form organic forms it is.
- **Curves are still approximated.** Spheres, tori, and other smooth surfaces must be expressed as polyhedral approximations made of primitives. The model does not natively understand curvature. (For shapes that are intrinsically curved, an SDF-based representation such as Fidget is a better fit and can coexist with this format.)
- **Variable size at one composition level requires care.** Within a single composite, primitives of mismatched dimensions can produce T-junctions at their interfaces. Rescaling the parent unit so all children become commensurate avoids this, but the format does not enforce it.

## Alternative Considered: Fixed-Cell Grid

A fixed-cell grid partitions space into a regular grid of identical cells. Each cell holds one entry from a palette (typically `{Empty, Slab, Wedge, Corner, Full}` plus an orientation). Adjacent cells share lattice points by construction, so vertex coincidence is automatic and T-junctions are structurally impossible. This is the model used by Minecraft, most voxel engines, and marching-cubes-style mesh extraction.

The fixed-cell model was rejected for this asset creator because:

- **Slopes are quantized to three angles.** Only 0°, 45°, and ≈54.7° are expressible from a single-cell primitive. Any other slope must be approximated as a staircase, which gives a "blocky" visual character incompatible with the intended asset style.
- **Symmetry must be replicated.** A symmetric shape must be written out once per cell; there is no way to say "this brick mirrored along X" in one declaration.
- **Variable scale requires either uniform fine cells (wasteful) or an octree (more bookkeeping than the equivalent stretchy-lego tree).**
- **The grid's main wins — free vertex sharing, free neighbor queries, structural T-junction prevention — are either not needed for static asset authoring or recoverable in the bounds-based format with cheap additions (e.g., a load-time adjacency check for T-junctions).**

The fixed-cell model would be the right choice if the primary use case were procedural generation from cell-fill rules, real-time per-cell simulation, or interactive 3D-pixel-grid editing. None of those are goals of this asset creator.

## T-junctions

T-junctions are the one structural advantage of the fixed-cell model that the stretchy-lego model does not get for free. They are the main known sharp edge of this format and warrant their own treatment, which will be added in a follow-up section.
