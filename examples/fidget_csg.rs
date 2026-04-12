//! Standalone SDF-based CSG using fidget: sphere intersected with a cube.
//! No polygon splitting, no BSP trees, no manifold issues.
//! The shapes stay mathematical until the final meshing step.
//!
//! Run with: cargo run --example fidget_csg

use fidget::context::Tree;
use fidget::vm::VmShape;
use fidget::mesh::{Octree, Settings};
use std::io::Write;

fn main() {
    let _ = std::fs::create_dir_all("generated/fidget-csg");

    println!("=== Fidget SDF-based CSG ===\n");

    // Define shapes as signed distance functions
    // Negative = inside, positive = outside

    // Sphere: sqrt(x² + y² + z²) - radius
    // Shapes sized to fit within [-1, 1]³ (fidget's octree default bounds)
    // Box with corners clipped by sphere.
    // Box half=0.6, corner distance = sqrt(3)*0.6 ≈ 1.04
    // Sphere radius=0.85 — cuts corners but not faces (0.6 < 0.85 < 1.04)
    let sphere_radius = 0.85;
    let sphere = (Tree::x().square() + Tree::y().square() + Tree::z().square()).sqrt()
        - sphere_radius;

    // Box: max(|x| - half, |y| - half, |z| - half)
    let box_half = 0.6;
    let cube = (Tree::x().abs() - box_half)
        .max(Tree::y().abs() - box_half)
        .max(Tree::z().abs() - box_half);

    // CSG operations on SDFs:
    // Union:     min(a, b)
    // Subtract:  max(a, -b)
    // Intersect: max(a, b)

    let intersect = sphere.clone().max(cube.clone());
    let subtract = sphere.clone().max(-cube.clone());
    let union = sphere.clone().min(cube.clone());

    let settings = Settings {
        depth: 5,
        ..Default::default()
    };

    // Mesh each result
    println!("Meshing sphere...");
    mesh_and_save(&sphere, &settings, "generated/fidget-csg/sphere.obj", "Sphere");

    println!("Meshing cube...");
    mesh_and_save(&cube, &settings, "generated/fidget-csg/cube.obj", "Cube");

    println!("Meshing intersect (sphere ∩ cube)...");
    mesh_and_save(&intersect, &settings, "generated/fidget-csg/intersect.obj", "Intersect");

    println!("Meshing subtract (sphere - cube)...");
    mesh_and_save(&subtract, &settings, "generated/fidget-csg/subtract.obj", "Subtract");

    println!("Meshing union (sphere ∪ cube)...");
    mesh_and_save(&union, &settings, "generated/fidget-csg/union.obj", "Union");

    println!("\nAll results saved to generated/fidget-csg/");
    println!("Open .obj files in any 3D viewer to inspect.");
}

fn mesh_and_save(tree: &Tree, settings: &Settings, path: &str, label: &str) {
    let shape = VmShape::from(tree.clone());
    let Some(octree) = Octree::build(&shape, settings) else {
        println!("  {label}: octree build returned None (cancelled?)");
        return;
    };
    let mesh = octree.walk_dual();

    println!("  {label}: {} vertices, {} triangles",
        mesh.vertices.len(), mesh.triangles.len());

    // Save as OBJ
    let mut f = std::fs::File::create(path).unwrap();
    for v in &mesh.vertices {
        writeln!(f, "v {} {} {}", v.x, v.y, v.z).unwrap();
    }
    for t in &mesh.triangles {
        writeln!(f, "f {} {} {}", t.x + 1, t.y + 1, t.z + 1).unwrap();
    }
}
