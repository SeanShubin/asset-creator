# Render Export System

The render export system automatically renders every `.shape.ron` file to a
PNG image in `generated/renders/`, keeping them synchronized as shapes are added,
modified, or deleted.

## How it works

1. **On startup**, scans `data/shapes/` and compares file modification times
   against `generated/renders/`. Queues any shape whose RON is newer than its PNG
   (or whose PNG doesn't exist).

2. **On shape change**, watches the registry's `shape_generation` counter.
   When it increments (a `.shape.ron` was modified on disk), re-scans for
   dirty shapes.

3. **Orphan cleanup**: PNGs in `generated/renders/` with no corresponding
   `.shape.ron` are deleted.

4. **Rendering**: processes one shape at a time. For each shape:
   - Creates an offscreen render target image (1024x1024, RGBA)
   - Spawns the shape with `spawn_shape_with_layers` on a dedicated render
     layer so it doesn't appear in the editor viewport
   - Spawns an orthographic camera (45/45 isometric) fitted to the shape's
     AABB with zero margin
   - Spawns a Bevy `Screenshot` observer that saves the result as PNG with
     alpha transparency preserved
   - Waits for the screenshot to complete, then cleans up all entities

## Camera fitting

The orthographic scale is computed precisely by projecting all 8 corners of
the shape's AABB through the camera's view matrix. The resulting screen-space
bounding box determines the exact scale needed to fill the image edge-to-edge.
No approximation or padding is used.

## Transparent backgrounds

The camera clears to `Color::NONE` (fully transparent). A custom save
function (`save_png_with_alpha`) preserves the alpha channel — Bevy's
built-in `save_to_disk` discards it.

As a safety measure, completely blank renders (all pixels transparent) are
not written to disk. This prevents a failed render from poisoning the mtime
cache and blocking future re-renders.

## Render layer isolation

Export shapes are rendered on `RenderLayers::layer(1)`. The editor camera
uses the default layer (0). This prevents:
- Export shapes appearing in the editor viewport
- Editor shapes appearing in export renders

All entities are assigned their render layer at creation time via
`spawn_shape_with_layers`, not via frame-delayed propagation.

## Bevy render timing constraint

Bevy's offscreen render-to-image pipeline has one timing constraint:

**Spawning entities on frame 0 (the very first frame of the application)
without RenderLayers produces a blank render.**

This was determined empirically using `examples/render_timing.rs`, which
tests every combination of spawn timing and render layer usage. The finding:

| Configuration                      | Result    |
| ---------------------------------- | --------- |
| Frame 0, no RenderLayers           | **BLANK** |
| Frame 0, with RenderLayers         | OK        |
| Frame 1+, no RenderLayers          | OK        |
| Frame 1+, with RenderLayers        | OK        |
| Screenshot deferred to later frame | OK        |
| Camera deferred to later frame     | OK        |

Since the export system always uses RenderLayers, this constraint is
automatically satisfied. No startup delay is needed.

To re-verify this constraint after Bevy upgrades:

```
cargo run --example render_timing
```
