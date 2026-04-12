//! Non-interactive stress test: renders every shape and logs resource usage stats.
//! Run with: cargo run -- --stress-test

use std::io::Write;

use crate::registry::AssetRegistry;
use crate::shape::{
    ShapeNode, walk_shape_tree, collect_mesh_from_events,
    ColorMap, CsgStats, CombineMode, ShapeEvent, perform_csg_pipeline,
};

const OUTPUT_DIR: &str = "generated/stress-test";
const LOG_FILE: &str = "generated/stress-test/stats.log";

pub fn is_stress_test() -> bool {
    std::env::args().any(|a| a == "--stress-test")
}

pub fn run(registry: &AssetRegistry) {
    let _ = std::fs::remove_dir_all(OUTPUT_DIR);
    let _ = std::fs::create_dir_all(OUTPUT_DIR);

    let mut log = std::fs::File::create(LOG_FILE).expect("cannot create log");
    writeln!(log, "=== Shape Stress Test ===\n").unwrap();

    let entries = registry.shape_entries();
    writeln!(log, "Shapes found: {}\n", entries.len()).unwrap();

    let mut any_warning = false;

    for (name, path) in &entries {
        let Some(shape) = registry.get_shape_by_path(path) else {
            writeln!(log, "SKIP {name}: not found in registry").unwrap();
            continue;
        };

        writeln!(log, "--- {name} ---").unwrap();

        // Walk the shape tree and collect events
        let colors: ColorMap = shape.palette.clone();
        let events = walk_shape_tree(shape, &colors, registry);

        let enter_count = events.iter().filter(|e| matches!(e, ShapeEvent::EnterNode { .. })).count();
        let geometry_count = events.iter().filter(|e| matches!(e, ShapeEvent::Geometry { .. })).count();
        writeln!(log, "  events: {} total, {} nodes, {} geometry", events.len(), enter_count, geometry_count).unwrap();

        // Collect mesh
        let mesh = collect_mesh_from_events(&events);
        writeln!(log, "  mesh: {} tris, {} verts", mesh.indices.len() / 3, mesh.positions.len()).unwrap();

        // Check for CSG children
        if shape.has_csg_children() {
            writeln!(log, "  CSG: yes (root level)").unwrap();
            let stats = run_csg_for_shape(shape, &colors, registry);
            log_csg_stats(&mut log, &stats, &mut any_warning);
        } else {
            // Check children recursively for CSG
            let csg_stats = find_and_run_csg(shape, &colors, registry);
            if !csg_stats.is_empty() {
                for (child_name, stats) in &csg_stats {
                    writeln!(log, "  CSG in child '{child_name}':").unwrap();
                    log_csg_stats(&mut log, stats, &mut any_warning);
                }
            }
        }

        // AABB
        if let Some(aabb) = shape.compute_aabb() {
            let size = aabb.size();
            writeln!(log, "  aabb: size=({:.1}, {:.1}, {:.1})", size.0, size.1, size.2).unwrap();
        }

        writeln!(log).unwrap();
    }

    // Summary
    writeln!(log, "=== Summary ===").unwrap();
    writeln!(log, "Shapes processed: {}", entries.len()).unwrap();
    if any_warning {
        writeln!(log, "WARNINGS: see above for shapes with high BSP depth or polygon count").unwrap();
    } else {
        writeln!(log, "All shapes within normal parameters").unwrap();
    }

    println!("Stress test complete. Results in {LOG_FILE}");
}

fn run_csg_for_shape(shape: &ShapeNode, colors: &ColorMap, registry: &AssetRegistry) -> CsgStats {
    let mut union_meshes = Vec::new();
    let mut subtract_meshes = Vec::new();
    let mut clip_meshes = Vec::new();

    for child in &shape.children {
        let child_events = walk_shape_tree(child, colors, registry);
        let raw = collect_mesh_from_events(&child_events);
        if raw.positions.is_empty() { continue; }
        match child.combine {
            CombineMode::Union => union_meshes.push(raw),
            CombineMode::Subtract => subtract_meshes.push(raw),
            CombineMode::Clip => clip_meshes.push(raw),
        }
    }

    let (_result, stats) = perform_csg_pipeline(union_meshes, subtract_meshes, clip_meshes);
    stats
}

fn find_and_run_csg(node: &ShapeNode, colors: &ColorMap, registry: &AssetRegistry) -> Vec<(String, CsgStats)> {
    let mut results = Vec::new();
    for child in &node.children {
        if child.has_csg_children() {
            let name = child.name.clone().unwrap_or_else(|| "unnamed".into());
            let stats = run_csg_for_shape(child, colors, registry);
            results.push((name, stats));
        }
        results.extend(find_and_run_csg(child, colors, registry));
    }
    results
}

fn log_csg_stats(log: &mut std::fs::File, stats: &CsgStats, any_warning: &mut bool) {
    writeln!(log, "    union inputs: {:?}", stats.input_union_tris).unwrap();
    if !stats.input_subtract_tris.is_empty() {
        writeln!(log, "    subtract inputs: {:?}", stats.input_subtract_tris).unwrap();
    }
    if !stats.input_clip_tris.is_empty() {
        writeln!(log, "    clip inputs: {:?}", stats.input_clip_tris).unwrap();
    }
    writeln!(log, "    max BSP depth: {}", stats.max_bsp_depth).unwrap();
    writeln!(log, "    max BSP polys: {}", stats.max_bsp_polys).unwrap();
    writeln!(log, "    max clip recursion: {}", stats.max_clip_recursion).unwrap();
    writeln!(log, "    output tris: {}", stats.output_tris).unwrap();

    if stats.max_bsp_depth > 100 {
        writeln!(log, "    ⚠ WARNING: BSP depth {} exceeds threshold 100", stats.max_bsp_depth).unwrap();
        *any_warning = true;
    }
    if stats.max_clip_recursion > 200 {
        writeln!(log, "    ⚠ WARNING: clip recursion {} exceeds threshold 200", stats.max_clip_recursion).unwrap();
        *any_warning = true;
    }
    if stats.max_bsp_polys > 10000 {
        writeln!(log, "    ⚠ WARNING: BSP polygon count {} exceeds threshold 10000", stats.max_bsp_polys).unwrap();
        *any_warning = true;
    }
}
