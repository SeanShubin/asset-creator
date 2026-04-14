//! SDF (Signed Distance Field) builders for shape primitives.
//! Each primitive is expressed as a fidget Tree — a mathematical function
//! of (x, y, z) where negative = inside, positive = outside.

use bevy::prelude::*;
use fidget::context::Tree;
use super::render::{RenderEvent, combine_transforms};
use super::spec::{Bounds, PrimitiveShape};

/// Build an SDF Tree from render events. The tree represents the combined
/// geometry of all Geometry events, positioned in world space.
pub fn collect_sdf_from_events(events: &[RenderEvent]) -> Option<Tree> {
    let mut sdf_stack: Vec<Option<Tree>> = vec![None];
    let mut tf_stack: Vec<Transform> = vec![Transform::IDENTITY];

    for event in events {
        match event {
            RenderEvent::EnterNode { local_tf, .. } => {
                let parent_world = *tf_stack.last().unwrap();
                let world = combine_transforms(&parent_world, local_tf);
                tf_stack.push(world);
                sdf_stack.push(None);
            }
            RenderEvent::AttachCsgGroup { .. } => {
                // Group metadata only; no SDF contribution.
            }
            RenderEvent::Geometry { shape, mesh_tf, .. } => {
                let parent_world = *tf_stack.last().unwrap();
                let world_mesh_tf = combine_transforms(&parent_world, mesh_tf);

                let sdf = primitive_sdf(*shape, &world_mesh_tf);
                let current = sdf_stack.last_mut().unwrap();
                *current = Some(match current.take() {
                    Some(existing) => existing.min(sdf),
                    None => sdf,
                });
            }
            RenderEvent::PrecomputedMesh { .. } => {
                // Pre-computed meshes can't be converted back to SDF.
                // This case doesn't arise in practice: CSG walks individual
                // children, which don't contain pre-computed mesh events.
            }
            RenderEvent::ExitNode => {
                tf_stack.pop();
                let child_sdf = sdf_stack.pop().unwrap();
                if let Some(child) = child_sdf {
                    let current = sdf_stack.last_mut().unwrap();
                    *current = Some(match current.take() {
                        Some(existing) => existing.min(child),
                        None => child,
                    });
                }
            }
        }
    }

    sdf_stack.pop().unwrap()
}

/// Build an SDF for a single primitive shape at its world transform.
fn primitive_sdf(shape: PrimitiveShape, world_tf: &Transform) -> Tree {
    // The world transform maps the unit mesh (-0.5..0.5) to world position.
    // For the SDF, we need the inverse: map world (x,y,z) to local coordinates,
    // then evaluate the unit SDF.
    let mat = world_tf.compute_matrix();
    let inv = mat.inverse();

    // Transform world coordinates to local
    let wx = Tree::x();
    let wy = Tree::y();
    let wz = Tree::z();

    // Mat4 is column-major: x_axis is column 0, y_axis is column 1, etc.
    // To compute local = inv * world_point, row i of the result is:
    //   col0[i]*wx + col1[i]*wy + col2[i]*wz + col3[i]
    let lx = wx.clone() * inv.x_axis.x + wy.clone() * inv.y_axis.x + wz.clone() * inv.z_axis.x + inv.w_axis.x;
    let ly = wx.clone() * inv.x_axis.y + wy.clone() * inv.y_axis.y + wz.clone() * inv.z_axis.y + inv.w_axis.y;
    let lz = wx * inv.x_axis.z + wy * inv.y_axis.z + wz * inv.z_axis.z + inv.w_axis.z;

    // Unit SDFs: shapes from -0.5 to 0.5
    match shape {
        PrimitiveShape::Box => sdf_box(lx, ly, lz),
        PrimitiveShape::Sphere => sdf_sphere(lx, ly, lz),
        PrimitiveShape::Cylinder => sdf_cylinder(lx, ly, lz),
        PrimitiveShape::Dome => sdf_dome(lx, ly, lz),
        PrimitiveShape::Cone => sdf_cone(lx, ly, lz),
        PrimitiveShape::Wedge => sdf_wedge(lx, ly, lz),
        PrimitiveShape::Torus => sdf_torus(lx, ly, lz),
        PrimitiveShape::Corner => sdf_corner(lx, ly, lz),
    }
}

// === Unit SDF functions (shapes from -0.5 to 0.5) ===

fn sdf_box(x: Tree, y: Tree, z: Tree) -> Tree {
    (x.abs() - 0.5).max(y.abs() - 0.5).max(z.abs() - 0.5)
}

fn sdf_sphere(x: Tree, y: Tree, z: Tree) -> Tree {
    (x.square() + y.square() + z.square()).sqrt() - 0.5
}

fn sdf_cylinder(x: Tree, y: Tree, z: Tree) -> Tree {
    // Cylinder along Y axis, radius 0.5, height 1.0
    let radial = (x.square() + z.square()).sqrt() - 0.5;
    let vertical = y.abs() - 0.5;
    radial.max(vertical)
}

fn sdf_dome(x: Tree, y: Tree, z: Tree) -> Tree {
    // Ellipsoidal cap matching the mesh builder's profile:
    // r = 0.5*cos(t), y = -0.5 + sin(t) for t in [0, PI/2]
    // This traces an ellipse: (r/0.5)² + (y+0.5)² = 1
    // Which is: 4(x² + z²) + (y+0.5)² = 1, clipped to y >= -0.5
    let scaled_xz = (x.square() + z.square()) * 4.0;
    let dy = y.clone() + 0.5;
    let ellipsoid = (scaled_xz + dy.square()).sqrt() - 1.0;
    let clip = -(y + 0.5); // y >= -0.5
    ellipsoid.max(clip)
}

fn sdf_cone(x: Tree, y: Tree, z: Tree) -> Tree {
    // Cone: base at y=-0.5 (radius 0.5), tip at y=0.5
    // Radius at height y: r = 0.5 * (0.5 - y)
    let r_at_y = (Tree::from(0.5) - y.clone()) * 0.5;
    let radial = (x.square() + z.square()).sqrt() - r_at_y;
    let cap = -y - 0.5; // y >= -0.5
    radial.max(cap)
}

fn sdf_wedge(x: Tree, y: Tree, z: Tree) -> Tree {
    // Wedge: box that tapers to zero height at z=0.5
    // Back face at z=-0.5, slope from (z=-0.5,y=0.5) to (z=0.5,y=-0.5)
    let bottom = -y.clone() - 0.5;
    let back = -z.clone() - 0.5;
    let left = -x.clone() - 0.5;
    let right = x - 0.5;
    // Slope plane: y + z <= 0 (normalized)
    let slope = (y + z) * std::f32::consts::FRAC_1_SQRT_2;
    bottom.max(back).max(left).max(right).max(slope)
}

fn sdf_torus(x: Tree, y: Tree, z: Tree) -> Tree {
    // Torus: major radius 0.35, minor radius 0.15
    let major_r = 0.35;
    let minor_r = 0.15;
    let xz_dist = (x.square() + z.square()).sqrt() - major_r;
    (xz_dist.square() + y.square()).sqrt() - minor_r
}

fn sdf_corner(x: Tree, y: Tree, z: Tree) -> Tree {
    // Tetrahedron in the (-x, -y, -z) corner
    let bottom = -y.clone() - 0.5;
    let back = -z.clone() - 0.5;
    let left = -x.clone() - 0.5;
    // Diagonal plane: x + y + z <= -0.5 → the cut face
    let diag = (x + y + z + 0.5) * (1.0 / 3.0_f32.sqrt());
    bottom.max(back).max(left).max(diag)
}

/// Mesh an SDF tree using fidget's octree, returning positions and indices.
/// The `scale` parameter converts the integer AABB into the same coordinate
/// space as the SDF (which divides by scale when building transforms).
pub fn mesh_sdf(tree: &Tree, bounds: &Bounds, scale: (i32, i32, i32)) -> (Vec<[f32; 3]>, Vec<u32>) {
    use fidget::vm::VmShape;
    use fidget::mesh::{Octree, Settings};

    let shape = VmShape::from(tree.clone());

    let raw_center = bounds.center_f32();
    let raw_size = bounds.size();
    let center = (
        raw_center.0 / scale.0 as f32,
        raw_center.1 / scale.1 as f32,
        raw_center.2 / scale.2 as f32,
    );
    let extent = (
        raw_size.0 as f32 / scale.0 as f32,
        raw_size.1 as f32 / scale.1 as f32,
        raw_size.2 as f32 / scale.2 as f32,
    );
    let max_extent = extent.0.max(extent.1).max(extent.2).max(0.001);
    let half = max_extent / 2.0;

    let world_to_model = nalgebra::Matrix4::new(
        half, 0.0, 0.0, center.0,
        0.0, half, 0.0, center.1,
        0.0, 0.0, half, center.2,
        0.0, 0.0, 0.0, 1.0,
    );

    let settings = Settings {
        depth: 6,
        world_to_model,
        ..Default::default()
    };

    let Some(octree) = Octree::build(&shape, &settings) else {
        return (vec![], vec![]);
    };
    let mesh = octree.walk_dual();

    let positions: Vec<[f32; 3]> = mesh.vertices.iter()
        .map(|v| [v.x, v.y, v.z])
        .collect();
    let indices: Vec<u32> = mesh.triangles.iter()
        .flat_map(|t| [t.x as u32, t.y as u32, t.z as u32])
        .collect();

    (positions, indices)
}
