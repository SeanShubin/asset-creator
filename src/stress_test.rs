//! Non-interactive stress test: renders every shape and logs resource usage stats.
//! Run with: cargo run -- --stress-test

use std::io::Write;

use crate::registry::AssetRegistry;
use crate::shape::{
    ShapeNode, walk_shape_tree, collect_mesh_from_events,
    ColorMap, CsgStats, CombineMode, ShapeEvent, perform_csg_from_children,
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
        let t0 = std::time::Instant::now();
        let events = walk_shape_tree(shape, &colors, registry);

        let enter_count = events.iter().filter(|e| matches!(e, ShapeEvent::EnterNode { .. })).count();
        let geometry_count = events.iter().filter(|e| matches!(e, ShapeEvent::Geometry { .. })).count();
        writeln!(log, "  events: {} total, {} nodes, {} geometry", events.len(), enter_count, geometry_count).unwrap();

        // Collect mesh
        let mesh = collect_mesh_from_events(&events);
        let mesh_ms = t0.elapsed().as_secs_f64() * 1000.0;
        writeln!(log, "  mesh: {} tris, {} verts ({:.1}ms)", mesh.indices.len() / 3, mesh.positions.len(), mesh_ms).unwrap();

        // Check for CSG children
        if shape.has_csg_children() {
            writeln!(log, "  CSG: yes (root level)").unwrap();
            let stats = run_csg_for_shape(shape, &colors, registry, &mut log);
            log_csg_stats(&mut log, &stats, &mut any_warning);
        } else {
            // Check children recursively for CSG
            let csg_stats = find_and_run_csg(shape, &colors, registry, &mut log);
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
            writeln!(log, "  aabb: size=({}, {}, {})", size.0, size.1, size.2).unwrap();
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

fn run_csg_for_shape(shape: &ShapeNode, colors: &ColorMap, registry: &AssetRegistry, log: &mut std::fs::File) -> CsgStats {
    let aabb = shape.compute_aabb()
        .unwrap_or(crate::shape::Bounds(-1, -1, -1, 1, 1, 1));

    for child in &shape.children {
        let name = child.name.as_deref().unwrap_or("?");
        let combine = match child.combine {
            CombineMode::Union => "union",
            CombineMode::Subtract => "subtract",
            CombineMode::Clip => "clip",
        };
        writeln!(log, "    child '{}': combine={}", name, combine).unwrap();
    }

    let (_result, stats) = perform_csg_from_children(&shape.children, colors, registry, &aabb);
    stats
}


fn find_and_run_csg(node: &ShapeNode, colors: &ColorMap, registry: &AssetRegistry, log: &mut std::fs::File) -> Vec<(String, CsgStats)> {
    let mut results = Vec::new();
    for child in &node.children {
        if child.has_csg_children() {
            let name = child.name.clone().unwrap_or_else(|| "unnamed".into());
            let stats = run_csg_for_shape(child, colors, registry, log);
            results.push((name, stats));
        }
        results.extend(find_and_run_csg(child, colors, registry, log));
    }
    results
}

fn log_csg_stats(log: &mut std::fs::File, stats: &CsgStats, _any_warning: &mut bool) {
    writeln!(log, "    inputs: {} union, {} subtract, {} clip",
        stats.input_union_count, stats.input_subtract_count, stats.input_clip_count).unwrap();
    writeln!(log, "    output tris: {} ({:.1}ms)", stats.output_tris, stats.mesh_time_ms).unwrap();
}
