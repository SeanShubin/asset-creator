use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: cargo run --example png_info <path.png>");
        std::process::exit(1);
    }

    let path = &args[1];
    let img = image::open(path).unwrap_or_else(|e| {
        eprintln!("Failed to open {path}: {e}");
        std::process::exit(1);
    });

    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let pixels = rgba.pixels().collect::<Vec<_>>();
    let total = pixels.len();

    let transparent = pixels.iter().filter(|p| p[3] == 0).count();
    let opaque = pixels.iter().filter(|p| p[3] == 255).count();
    let partial = total - transparent - opaque;

    println!("File: {path}");
    println!("Size: {w}x{h}");
    println!("Total pixels: {total}");
    println!("Transparent (alpha=0): {transparent} ({:.1}%)", transparent as f64 / total as f64 * 100.0);
    println!("Opaque (alpha=255): {opaque} ({:.1}%)", opaque as f64 / total as f64 * 100.0);
    println!("Partial (0<alpha<255): {partial} ({:.1}%)", partial as f64 / total as f64 * 100.0);

    // Sample some edge pixels to verify transparency
    let corners = [
        (0, 0, "top-left"),
        (w - 1, 0, "top-right"),
        (0, h - 1, "bottom-left"),
        (w - 1, h - 1, "bottom-right"),
    ];
    println!("\nCorner pixels:");
    for (x, y, name) in corners {
        let p = rgba.get_pixel(x, y);
        println!("  {name}: rgba({}, {}, {}, {})", p[0], p[1], p[2], p[3]);
    }
}
