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

A T-junction in a polygon mesh is a vertex from one face that lands on the **interior of an edge** of another face, rather than at one of that edge's endpoints. The shape resembles the letter T: a short edge meeting a long edge at its middle. In 3D this happens wherever primitives of different sizes share a face — the smaller primitive's corner sits in the interior of the larger primitive's edge.

T-junctions are geometrically valid but break invariants that rendering and mesh-processing pipelines rely on:

- **Rendering cracks.** Floating-point rasterization of a long edge can place the surface a sub-pixel away from where the meeting short edges land, producing flickering one-pixel-wide gaps along the seam.
- **Interpolation discontinuities.** Per-vertex normals, colors, and UVs interpolated along the long edge do not match the values computed along the meeting short edges, even though the geometric point is the same.
- **Manifold violation.** The long edge is shared by one face on its outer side and two faces on its inner side, breaking the invariant that every edge has exactly two adjacent faces. Subdivision, remeshing, half-edge data structures, and clean CSG all assume this invariant.
- **Boolean / CSG breakage.** Boundary points without a consistent local manifold structure cause boolean operations to either reject the input or produce self-intersecting output.

A fixed-cell grid prevents T-junctions structurally because adjacent cells are always the same size and share entire faces. The stretchy-lego model permits T-junctions because variable per-axis stretching means adjacent primitives can have mismatched face dimensions. Three concrete ways they arise:

1. **Mismatched stretch on adjacent boxes.** A `4×4×4` Box adjacent to two stacked `4×2×4` Boxes produces a T-junction at the midpoint of the big Box's shared edge.
2. **Stretched Wedge against an unstretched Box.** A Wedge with a `1×3` rectangular face meeting a Box with a `1×1` face produces T-junctions where the smaller face's corners land on the longer face's edge.
3. **Recursive composition with mismatched scales.** Two composites at different unit scales sharing a face have all their boundary vertices on a finer lattice than their neighbor, even when each composite is internally clean.

## Resolution: integer-exact fusion at meshing time

T-junctions are removed by a three-pass meshing step that operates on the raw mesh produced by concatenating primitive meshes. Because the source format keeps all coordinates in exact integer arithmetic and primitives meet at shared faces without overlapping interiors, all three passes stay in the integer domain:

1. **Vertex weld.** Collect every primitive's mesh vertices and merge any two whose integer coordinates are bit-identical. "Coincident" means literal equality, not "within tolerance." After this pass, two primitives that share a face reference the same vertex objects on that face.
2. **Internal face cancellation.** For every face, look for another face with the identical vertex set and opposite winding order. Such pairs are internal seams between two filled primitives. Delete both. After this pass, every surviving face is on the outer boundary of the union.
3. **T-junction repair.** For every edge, look for any vertex whose integer coordinates lie on the interior of that edge (an exact integer collinearity test). Split the edge at the vertex and split any face using the edge into two faces sharing the new vertex. Repeat until no T-junctions remain.

The result is a single watertight 2-manifold with all-integer vertex coordinates, no internal faces, no T-junctions, no duplicated vertices, and bit-exact correspondence to the source. The integer domain is never left.

This reframes T-junctions as a meshing concern rather than a format concern. The `.shape.ron` source files stay simple, declarative, and integer-exact. Authors are free to compose primitives at any compatible scales without worrying about T-junctions. The render path, export path, and collision-mesh path all run the three-pass meshing step and consume only its output.

### What this approach assumes and what it gives up

The three-pass approach assumes **primitives meet but never overlap**. Two primitives whose bounds touch along a face are fine; two primitives whose bounds share interior volume are not — pass 2 will not cancel their faces (which are nested rather than coincident) and the output mesh will contain stray internal geometry. A load-time validator that refuses any composite with overlapping primitive interiors enforces this invariant cheaply.

The other thing given up is **per-primitive identity in the meshed output**. After welding and face-cancellation, a triangle in the output cannot be traced back to a specific source primitive without explicit bookkeeping. If per-primitive colors or materials are needed, the meshing pass must tag each triangle with its source primitive before welding, and pass 2 must refuse to cancel face pairs that come from primitives with different materials — leaving those as visible internal seams. This is a small extension, not a redesign.

For genuinely overlapping primitives (which the format does not currently produce, and which the load-time validator can forbid), full CSG would be required. The asset-creator does not need this case.
