use super::hash::hash2d;

/// Cellular (Voronoi F2-F1) noise. Returns values roughly in -1..1.
pub fn cellular2d(x: f64, y: f64, seed: u32) -> f64 {
    let ix = x.floor() as i32;
    let iy = y.floor() as i32;

    let (min_dist, second_dist) = nearest_two_distances(x, y, ix, iy, seed);
    (second_dist - min_dist).clamp(0.0, 1.0) * 2.0 - 1.0
}

fn nearest_two_distances(x: f64, y: f64, ix: i32, iy: i32, seed: u32) -> (f64, f64) {
    let mut min_dist = f64::MAX;
    let mut second_dist = f64::MAX;

    for dy in -1..=1 {
        for dx in -1..=1 {
            let dist = cell_point_distance(x, y, ix + dx, iy + dy, seed);
            if dist < min_dist {
                second_dist = min_dist;
                min_dist = dist;
            } else if dist < second_dist {
                second_dist = dist;
            }
        }
    }

    (min_dist, second_dist)
}

fn cell_point_distance(x: f64, y: f64, cx: i32, cy: i32, seed: u32) -> f64 {
    let px = cx as f64 + hash2d(cx, cy, seed) * 0.8 + 0.1;
    let py = cy as f64 + hash2d(cx, cy, seed.wrapping_add(1)) * 0.8 + 0.1;
    ((x - px).powi(2) + (y - py).powi(2)).sqrt()
}
