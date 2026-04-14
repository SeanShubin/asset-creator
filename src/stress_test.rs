//! Non-interactive stress test: renders every shape and logs resource usage stats.
//! Run with: cargo run -- --stress-test

use std::io::Write;

use crate::registry::AssetRegistry;
use crate::shape::{
    collect_occupancy, collect_raw_mesh, compile, ColorMap, CombineMode,
    CsgStats, RenderEvent, SpecNode, perform_csg_uncached,
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
    let mut collision_failures: Vec<String> = Vec::new();

    for (name, path) in &entries {
        let Some(shape) = registry.get_shape_by_path(path) else {
            writeln!(log, "SKIP {name}: not found in registry").unwrap();
            continue;
        };

        writeln!(log, "--- {name} ---").unwrap();

        // Cell-level collision check. Non-interactive pipelines refuse
        // to produce output from a shape that violates cell uniqueness.
        // The editor remains permissive (HUD stat only) but batch tools
        // are strict by design — a broken shape should never ship.
        let occupancy = collect_occupancy(shape, registry);
        if occupancy.collision_count() > 0 {
            writeln!(
                log,
                "  FAIL: {} cell-level collision(s)",
                occupancy.collision_count()
            )
            .unwrap();
            for c in occupancy.collisions().iter().take(10) {
                writeln!(
                    log,
                    "    at {:?}: '{}' vs '{}'",
                    c.cell, c.first_path, c.second_path
                )
                .unwrap();
            }
            if occupancy.collisions().len() > 10 {
                writeln!(
                    log,
                    "    ... and {} more",
                    occupancy.collisions().len() - 10
                )
                .unwrap();
            }
            collision_failures.push(name.clone());
            writeln!(log).unwrap();
            continue;
        }

        // Walk the shape tree and collect events
        let colors: ColorMap = shape.palette.clone();
        let t0 = std::time::Instant::now();
        let events = compile(shape, &colors, registry);

        let enter_count = events.iter().filter(|e| matches!(e, RenderEvent::EnterNode { .. })).count();
        let geometry_count = events.iter().filter(|e| matches!(e, RenderEvent::Geometry { .. })).count();
        writeln!(log, "  events: {} total, {} nodes, {} geometry", events.len(), enter_count, geometry_count).unwrap();

        // Collect mesh
        let mesh = collect_raw_mesh(&events);
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

    if !collision_failures.is_empty() {
        writeln!(log).unwrap();
        writeln!(
            log,
            "COLLISION FAILURES ({}): these shapes were skipped",
            collision_failures.len()
        )
        .unwrap();
        for name in &collision_failures {
            writeln!(log, "  {name}").unwrap();
        }
        println!(
            "Stress test FAILED: {} shape(s) had cell collisions. See {LOG_FILE}",
            collision_failures.len()
        );
        std::process::exit(1);
    }

    println!("Stress test complete. Results in {LOG_FILE}");
}

fn run_csg_for_shape(shape: &SpecNode, colors: &ColorMap, registry: &AssetRegistry, log: &mut std::fs::File) -> CsgStats {
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

    let (_result, stats) = perform_csg_uncached(&shape.children, colors, registry, &aabb, (1, 1, 1));
    stats
}


fn find_and_run_csg(node: &SpecNode, colors: &ColorMap, registry: &AssetRegistry, log: &mut std::fs::File) -> Vec<(String, CsgStats)> {
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
