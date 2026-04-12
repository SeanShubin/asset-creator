# CSG Mesh Normals

Fidget's dual contouring produces a triangle mesh with shared vertices and
no normals. Converting this to a rendering mesh requires computing normals.
Two approaches fail; one works.

## What doesn't work

### Triangle cross product normals (black triangles)

Computing face normals from `(b-a) × (c-a)` produces correct normal
directions for most triangles, but dual contouring can produce triangles
with inconsistent winding order. Some face normals point inward, appearing
black because they face away from the light. There is no guarantee that
dual contouring output has consistent winding.

### SDF gradient normals (crinkly edges)

The SDF gradient at a vertex gives the mathematically correct outward
normal for the surface at that point. However, using per-vertex gradient
normals produces smooth interpolation across flat faces. At sharp edges
(box edges, cylinder rims), adjacent flat faces share vertices whose
gradient normals are averaged, producing a wavy/crinkly appearance instead
of clean flat faces.

## What works

### Flat face normals oriented by SDF gradient

1. Compute the face normal from the triangle cross product (flat shading)
2. Evaluate the SDF gradient at the three vertices
3. If the face normal points opposite to the average gradient, flip it

This gives flat shading (correct for planar faces) with correct outward
orientation (from the SDF gradient). Each triangle gets its own vertices
(unshared) so normals don't bleed between faces.

```rust
let cross = (b - a).cross(c - a);
let mut face_n = cross.normalize();

let avg_sdf_n = sdf_gradient(v0) + sdf_gradient(v1) + sdf_gradient(v2);
if face_n.dot(avg_sdf_n) < 0.0 {
    face_n = -face_n;
}
```
