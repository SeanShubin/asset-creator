//! One-off migration: read .shape.ron files with old shape/orient syntax,
//! print the equivalent new faces/corner/clip syntax.
//!
//! Usage: cargo run --example migrate_shapes

use std::path::Path;

/// The old SymOp values (copied from the codebase).
#[derive(Debug, Clone, Copy)]
enum SymOp {
    MirrorX, MirrorY, MirrorZ,
    Rotate90_XY, Rotate90_XZ, Rotate90_YZ,
    Rotate180_XY, Rotate180_XZ, Rotate180_YZ,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SA { PosX, NegX, PosY, NegY, PosZ, NegZ }

#[derive(Debug, Clone, Copy)]
struct Placement(SA, SA, SA);

fn identity() -> Placement { Placement(SA::PosX, SA::PosY, SA::PosZ) }

fn to_placement(op: SymOp) -> Placement {
    use SA::*;
    match op {
        SymOp::MirrorX     => Placement(NegX, PosY, PosZ),
        SymOp::MirrorY     => Placement(PosX, NegY, PosZ),
        SymOp::MirrorZ     => Placement(PosX, PosY, NegZ),
        SymOp::Rotate90_XY => Placement(NegY, PosX, PosZ),
        SymOp::Rotate90_XZ => Placement(NegZ, PosY, PosX),
        SymOp::Rotate90_YZ => Placement(PosX, NegZ, PosY),
        SymOp::Rotate180_XY => Placement(NegX, NegY, PosZ),
        SymOp::Rotate180_XZ => Placement(NegX, PosY, NegZ),
        SymOp::Rotate180_YZ => Placement(PosX, NegY, NegZ),
    }
}

fn compose(outer: Placement, inner: Placement) -> Placement {
    let pick = |sa: SA, p: &Placement| -> SA {
        let (idx, neg) = match sa {
            SA::PosX => (0, false), SA::NegX => (0, true),
            SA::PosY => (1, false), SA::NegY => (1, true),
            SA::PosZ => (2, false), SA::NegZ => (2, true),
        };
        let picked = match idx { 0 => p.0, 1 => p.1, _ => p.2 };
        if neg { negate(picked) } else { picked }
    };
    Placement(pick(outer.0, &inner), pick(outer.1, &inner), pick(outer.2, &inner))
}

fn negate(sa: SA) -> SA {
    match sa {
        SA::PosX => SA::NegX, SA::NegX => SA::PosX,
        SA::PosY => SA::NegY, SA::NegY => SA::PosY,
        SA::PosZ => SA::NegZ, SA::NegZ => SA::PosZ,
    }
}

fn compose_ops(ops: &[SymOp]) -> Placement {
    let mut result = identity();
    for &op in ops {
        result = compose(to_placement(op), result);
    }
    result
}

/// Convert a Placement to face notation for Corner/InverseCorner.
/// The identity corner fills vertex (-0.5,-0.5,-0.5) = (min,min,min).
/// Through the placement, this vertex maps to a specific world corner.
/// Each world axis's face is determined by which identity axis feeds it
/// and whether the sign is preserved or negated.
fn placement_to_corner_faces(p: Placement) -> String {
    // For each world axis, the placement tells us which identity axis
    // feeds it and with what sign. The identity corner vertex is at
    // -0.5 on all axes. If the sign is positive, -0.5 stays negative → Min.
    // If negated, -0.5 becomes +0.5 → Max.
    let face = |sa: SA, world_axis: &str| -> String {
        let is_pos = matches!(sa, SA::PosX | SA::PosY | SA::PosZ);
        if is_pos {
            format!("Min{}", world_axis)
        } else {
            format!("Max{}", world_axis)
        }
    };
    format!("({}, {}, {})", face(p.0, "X"), face(p.1, "Y"), face(p.2, "Z"))
}

/// Convert a Placement to face notation for Wedge.
/// Identity wedge fills MinY+MinZ (y+z ≤ 0), ridge along X.
/// The filled side has identity_y = -0.5 and identity_z = -0.5.
/// Through the placement, find which world faces these map to.
fn placement_to_wedge_faces(p: Placement) -> String {
    // For each world axis, the placement tells us which identity axis
    // feeds it. We need the world axes that receive identity_Y and
    // identity_Z (the cut axes), and their signs.
    let slots = [p.0, p.1, p.2];
    let world_names = ["X", "Y", "Z"];
    let mut faces = Vec::new();

    for (world_idx, &sa) in slots.iter().enumerate() {
        let (id_axis, is_pos) = match sa {
            SA::PosX => (0, true), SA::NegX => (0, false),
            SA::PosY => (1, true), SA::NegY => (1, false),
            SA::PosZ => (2, true), SA::NegZ => (2, false),
        };
        // Identity Y (axis 1) and Z (axis 2) are the cut axes.
        // Identity X (axis 0) is the ridge — skip it.
        if id_axis == 0 { continue; }

        // The identity vertex at -0.5 on this axis. If sign is positive,
        // -0.5 stays min. If negated, becomes max.
        let face = if is_pos {
            format!("Min{}", world_names[world_idx])
        } else {
            format!("Max{}", world_names[world_idx])
        };
        faces.push(face);
    }

    format!("({}, {})", faces[0], faces[1])
}

fn parse_orient(text: &str) -> Vec<SymOp> {
    let mut ops = Vec::new();
    for token in text.split(|c: char| c == '[' || c == ']' || c == ',' || c.is_whitespace()) {
        let token = token.trim();
        match token {
            "MirrorX" => ops.push(SymOp::MirrorX),
            "MirrorY" => ops.push(SymOp::MirrorY),
            "MirrorZ" => ops.push(SymOp::MirrorZ),
            "Rotate90_XY" => ops.push(SymOp::Rotate90_XY),
            "Rotate90_XZ" => ops.push(SymOp::Rotate90_XZ),
            "Rotate90_YZ" => ops.push(SymOp::Rotate90_YZ),
            "Rotate180_XY" => ops.push(SymOp::Rotate180_XY),
            "Rotate180_XZ" => ops.push(SymOp::Rotate180_XZ),
            "Rotate180_YZ" => ops.push(SymOp::Rotate180_YZ),
            _ => {}
        }
    }
    ops
}

fn main() {
    let shapes_dir = Path::new("data/shapes");
    let mut files: Vec<_> = walkdir(shapes_dir);
    files.sort();

    for path in &files {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let mut has_old_shape = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("shape:") && !trimmed.contains("Box") {
                has_old_shape = true;
                break;
            }
        }
        if !has_old_shape { continue; }

        println!("\n=== {} ===", path.display());

        // Parse line by line, tracking current shape+orient for each node.
        let mut current_name = String::new();
        let mut current_shape = String::new();
        let mut current_orient = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("name:") {
                current_name = trimmed.to_string();
                current_shape.clear();
                current_orient.clear();
            }
            if trimmed.starts_with("shape:") {
                current_shape = trimmed
                    .trim_start_matches("shape:")
                    .trim()
                    .trim_end_matches(',')
                    .to_string();
            }
            if trimmed.starts_with("orient:") {
                current_orient = parse_orient(trimmed);
            }

            // When we hit a closing paren or comma after shape, emit the translation.
            if (trimmed == ")," || trimmed == ")") && !current_shape.is_empty() {
                let placement = compose_ops(&current_orient);

                let new_field = match current_shape.as_str() {
                    "Wedge" => format!("faces: {}", placement_to_wedge_faces(placement)),
                    "Corner" => format!("corner: {}", placement_to_corner_faces(placement)),
                    "InverseCorner" => format!("clip: {}", placement_to_corner_faces(placement)),
                    _ => format!("??? shape={}", current_shape),
                };

                println!("  {} → {}", current_name, new_field);
                current_shape.clear();
                current_orient.clear();
            }
        }
    }
}

fn walkdir(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(walkdir(&path));
            } else if path.extension().is_some_and(|e| e == "ron") {
                files.push(path);
            }
        }
    }
    files
}
