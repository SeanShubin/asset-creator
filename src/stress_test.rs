//! Non-interactive stress test: compiles every shape and logs stats.
//! Run with: cargo run -- --stress-test

use std::io::Write;

use crate::registry::AssetRegistry;
use crate::shape::{collect_occupancy, compile};

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

    let mut collision_failures: Vec<String> = Vec::new();

    for (name, path) in &entries {
        let Some(shape) = registry.get_shape_by_path(path) else {
            writeln!(log, "SKIP {name}: not found in registry").unwrap();
            continue;
        };

        writeln!(log, "--- {name} ---").unwrap();

        // Cell-level collision check. Non-interactive pipelines refuse
        // to produce output from a shape that violates cell uniqueness.
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

        // Compile the shape into its CompiledShape tree and report
        // summary metrics about the fused meshes.
        let t0 = std::time::Instant::now();
        let compiled = compile(shape, registry);
        let compile_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let mut total_parts = 0usize;
        let mut total_meshes = 0usize;
        let mut total_tris = 0usize;
        let mut total_verts = 0usize;
        count_compiled(&compiled, &mut total_parts, &mut total_meshes, &mut total_tris, &mut total_verts);
        writeln!(
            log,
            "  compile: {} parts, {} fused meshes, {} tris, {} verts ({:.2}ms)",
            total_parts, total_meshes, total_tris, total_verts, compile_ms
        )
        .unwrap();

        if let Some(aabb) = occupancy.aabb() {
            let size = aabb.size();
            writeln!(log, "  aabb: size=({}, {}, {})", size.0, size.1, size.2).unwrap();
        }

        writeln!(log).unwrap();
    }

    // Summary
    writeln!(log, "=== Summary ===").unwrap();
    writeln!(log, "Shapes processed: {}", entries.len()).unwrap();

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

fn count_compiled(
    node: &crate::shape::CompiledShape,
    parts: &mut usize,
    meshes: &mut usize,
    tris: &mut usize,
    verts: &mut usize,
) {
    *parts += 1;
    for m in &node.meshes {
        *meshes += 1;
        *tris += m.mesh.indices.len() / 3;
        *verts += m.mesh.positions.len();
    }
    for child in &node.children {
        count_compiled(child, parts, meshes, tris, verts);
    }
}
