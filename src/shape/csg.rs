use bevy::prelude::*;
use super::definition::{Axis, Bounds, CsgOp, Combinator, PrimitiveShape, RepeatSpec, ShapeNode, reflect_orient};
use super::meshes::{RawMesh, create_raw_mesh};
use crate::registry::AssetRegistry;
use crate::util::Color3;

// =====================================================================
// Public API — called from interpreter
// =====================================================================

type ColorMap = Vec<(String, Color3)>;

/// Collect all geometry from a ShapeNode subtree into a single RawMesh.
/// Handles combinators (mirror/repeat/import/nested CSG) recursively.
/// The returned mesh has all transforms baked into vertex positions.
pub fn collect_node_mesh(
    node: &ShapeNode,
    parent_tf: Transform,
    colors: &ColorMap,
    registry: &AssetRegistry,
) -> RawMesh {
    let colors = if node.palette.is_empty() {
        colors.clone()
    } else {
        merge_colors(colors, &node.palette)
    };

    match node.combinator() {
        Combinator::Csg(op) => {
            collect_csg(node, parent_tf, op, &colors, registry)
        }
        Combinator::Mirror(axes) => {
            collect_mirror(node, parent_tf, axes, &colors, registry)
        }
        Combinator::Repeat(repeat) => {
            collect_repeat(node, parent_tf, repeat, &colors, registry)
        }
        Combinator::Import(name) => {
            collect_import(node, parent_tf, name, &colors, registry)
        }
        Combinator::None => {
            collect_leaf(node, parent_tf, &colors, registry)
        }
    }
}

/// Perform a CSG operation on the children of a CSG node.
/// Returns a single merged RawMesh.
pub fn perform_csg(op: &CsgOp, meshes: Vec<RawMesh>) -> RawMesh {
    if meshes.is_empty() {
        return RawMesh { positions: vec![], normals: vec![], uvs: vec![], indices: vec![] };
    }

    let mut iter = meshes.into_iter();
    let first = iter.next().unwrap();
    let mut result = mesh_to_bsp(first);

    for mesh in iter {
        let operand = mesh_to_bsp(mesh);
        result = match op {
            CsgOp::Union => bsp_union(result, operand),
            CsgOp::Subtract => bsp_subtract(result, operand),
            CsgOp::Intersect => bsp_intersect(result, operand),
        };
    }

    bsp_to_mesh(result)
}

// =====================================================================
// Mesh collection for each combinator type
// =====================================================================

fn collect_csg(
    node: &ShapeNode,
    parent_tf: Transform,
    op: &CsgOp,
    colors: &ColorMap,
    registry: &AssetRegistry,
) -> RawMesh {
    let child_meshes: Vec<RawMesh> = node.children.iter()
        .map(|child| collect_node_mesh(child, parent_tf, colors, registry))
        .collect();
    perform_csg(op, child_meshes)
}

fn collect_leaf(
    node: &ShapeNode,
    parent_tf: Transform,
    colors: &ColorMap,
    registry: &AssetRegistry,
) -> RawMesh {
    let mut result = RawMesh { positions: vec![], normals: vec![], uvs: vec![], indices: vec![] };

    // Attach this node's own geometry
    if let Some(shape) = &node.shape {
        let bounds = node.bounds.unwrap_or(Bounds(-0.5, -0.5, -0.5, 0.5, 0.5, 0.5));
        let mesh_tf = mesh_transform(*shape, &bounds, &node.orient);
        // Combine parent transform with mesh transform
        let world_tf = combine_transforms(&parent_tf, &mesh_tf);
        let mut raw = create_raw_mesh(*shape);
        raw.apply_transform(&world_tf);
        result.merge(&raw);
    }

    // Collect children
    let child_tf = build_child_transform(node, &parent_tf);
    for child in &node.children {
        let child_mesh = collect_node_mesh(child, child_tf, colors, registry);
        result.merge(&child_mesh);
    }

    result
}

fn collect_mirror(
    node: &ShapeNode,
    parent_tf: Transform,
    axes: &[Axis],
    colors: &ColorMap,
    registry: &AssetRegistry,
) -> RawMesh {
    let mut result = RawMesh { positions: vec![], normals: vec![], uvs: vec![], indices: vec![] };
    let mut base = node.clone();
    base.mirror = Vec::new();

    let combinations = mirror_combinations(axes);
    for (flipped_axes, _suffix) in &combinations {
        let mut copy = base.clone();
        for &axis in flipped_axes {
            flip_node_bounds(&mut copy, axis);
        }
        for &axis in flipped_axes {
            reflect_orientation(&mut copy, axis);
        }
        let child_mesh = collect_node_mesh(&copy, parent_tf, colors, registry);
        result.merge(&child_mesh);
    }

    result
}

fn collect_repeat(
    node: &ShapeNode,
    parent_tf: Transform,
    repeat: &RepeatSpec,
    colors: &ColorMap,
    registry: &AssetRegistry,
) -> RawMesh {
    let mut result = RawMesh { positions: vec![], normals: vec![], uvs: vec![], indices: vec![] };

    let start = if repeat.center {
        -(repeat.count as f32 - 1.0) * repeat.spacing * 0.5
    } else {
        0.0
    };

    for i in 0..repeat.count {
        let mut instance = node.clone();
        instance.repeat = None;
        reify_bounds(&mut instance);
        offset_bounds(&mut instance.bounds, repeat.along, start + i as f32 * repeat.spacing);
        let child_mesh = collect_node_mesh(&instance, parent_tf, colors, registry);
        result.merge(&child_mesh);
    }

    result
}

fn collect_import(
    node: &ShapeNode,
    parent_tf: Transform,
    import_name: &str,
    colors: &ColorMap,
    registry: &AssetRegistry,
) -> RawMesh {
    let imported = match registry.get_shape(import_name) {
        Some(shape) => shape.clone(),
        None => {
            warn!("CSG: Import '{}' not found in registry", import_name);
            return RawMesh { positions: vec![], normals: vec![], uvs: vec![], indices: vec![] };
        }
    };

    let native_aabb = imported.compute_aabb()
        .unwrap_or(Bounds(-0.5, -0.5, -0.5, 0.5, 0.5, 0.5));
    let placement = node.bounds.unwrap_or(native_aabb);

    let mut remapped = imported;
    remapped.remap_bounds(&native_aabb, &placement);

    collect_node_mesh(&remapped, parent_tf, colors, registry)
}

// =====================================================================
// Transform helpers (mirrored from interpreter.rs)
// =====================================================================

fn mesh_transform(shape: PrimitiveShape, bounds: &Bounds, om: &Mat3) -> Transform {
    let size = bounds.size();

    let local_x_size = pick_size_for_direction(om.x_axis, size);
    let local_y_size = pick_size_for_direction(om.y_axis, size);
    let local_z_size = pick_size_for_direction(om.z_axis, size);

    let local_scale = match shape {
        PrimitiveShape::Torus => Vec3::new(local_x_size, local_y_size / 0.3, local_z_size),
        _ => Vec3::new(local_x_size, local_y_size, local_z_size),
    };

    let col_x = om.x_axis * local_scale.x;
    let col_y = om.y_axis * local_scale.y;
    let col_z = om.z_axis * local_scale.z;

    let mat = Mat3::from_cols(col_x, col_y, col_z);
    let affine = bevy::math::Affine3A::from_mat3(mat);
    Transform::from_matrix(bevy::math::Mat4::from(affine))
}

fn pick_size_for_direction(dir: Vec3, size: (f32, f32, f32)) -> f32 {
    if dir.x.abs() > 0.5 { size.0 }
    else if dir.y.abs() > 0.5 { size.1 }
    else { size.2 }
}

fn build_child_transform(node: &ShapeNode, parent_tf: &Transform) -> Transform {
    let is_combinator = node.is_combinator();
    let position = if is_combinator {
        Vec3::ZERO
    } else {
        bounds_center(&node.bounds)
    };

    let mut tf = Transform::from_translation(position);
    if let Some((degrees, axis)) = node.rotate {
        let rad = degrees.to_radians();
        tf.rotation = match axis {
            Axis::X => Quat::from_rotation_x(rad),
            Axis::Y => Quat::from_rotation_y(rad),
            Axis::Z => Quat::from_rotation_z(rad),
        };
    }

    combine_transforms(parent_tf, &tf)
}

fn combine_transforms(parent: &Transform, child: &Transform) -> Transform {
    let parent_mat = parent.compute_matrix();
    let child_mat = child.compute_matrix();
    Transform::from_matrix(parent_mat * child_mat)
}

fn bounds_center(bounds: &Option<Bounds>) -> Vec3 {
    match bounds {
        Some(b) => {
            let c = b.center();
            Vec3::new(c.0, c.1, c.2)
        }
        None => Vec3::ZERO,
    }
}

fn reify_bounds(node: &mut ShapeNode) {
    if node.bounds.is_none() && node.shape.is_some() {
        node.bounds = Some(Bounds(-0.5, -0.5, -0.5, 0.5, 0.5, 0.5));
    }
}

fn offset_bounds(bounds: &mut Option<Bounds>, axis: Axis, offset: f32) {
    if let Some(ref mut b) = bounds {
        match axis {
            Axis::X => { b.0 += offset; b.3 += offset; }
            Axis::Y => { b.1 += offset; b.4 += offset; }
            Axis::Z => { b.2 += offset; b.5 += offset; }
        }
    }
}

fn flip_node_bounds(node: &mut ShapeNode, axis: Axis) {
    reify_bounds(node);
    if let Some(ref mut b) = node.bounds {
        match axis {
            Axis::X => { let tmp = -b.0; b.0 = -b.3; b.3 = tmp; }
            Axis::Y => { let tmp = -b.1; b.1 = -b.4; b.4 = tmp; }
            Axis::Z => { let tmp = -b.2; b.2 = -b.5; b.5 = tmp; }
        }
    }
    for child in &mut node.children {
        flip_node_bounds(child, axis);
    }
}

fn reflect_orientation(node: &mut ShapeNode, axis: Axis) {
    if node.shape.is_some() {
        reflect_orient(&mut node.orient, axis);
    }
    for child in &mut node.children {
        reflect_orientation(child, axis);
    }
}

fn mirror_combinations(axes: &[Axis]) -> Vec<(Vec<Axis>, String)> {
    let n = axes.len();
    let count = 1 << n;
    let mut result = Vec::with_capacity(count);
    for bits in 0..count {
        let mut flipped = Vec::new();
        let mut suffix = String::new();
        for (i, &axis) in axes.iter().enumerate() {
            if bits & (1 << i) != 0 {
                flipped.push(axis);
                let letter = match axis { Axis::X => "x", Axis::Y => "y", Axis::Z => "z" };
                suffix.push_str(letter);
            }
        }
        let suffix = if suffix.is_empty() { String::new() } else { format!("m{suffix}") };
        result.push((flipped, suffix));
    }
    result
}

fn merge_colors(parent: &ColorMap, child: &ColorMap) -> ColorMap {
    let mut merged = child.clone();
    for (pk, pv) in parent {
        if let Some(entry) = merged.iter_mut().find(|(k, _)| k == pk) {
            entry.1 = *pv;
        } else {
            merged.push((pk.clone(), *pv));
        }
    }
    merged
}

// =====================================================================
// BSP-based CSG implementation
// =====================================================================
//
// Based on the classic algorithm from Naylor (1990) and popularized by
// csg.js (Evan Wallace). Each solid is represented as a BSP tree of
// polygons. Boolean operations clip polygons against the opposing tree.

const EPSILON: f32 = 1e-5;

#[derive(Clone, Debug)]
struct Vertex {
    pos: Vec3,
    normal: Vec3,
    uv: [f32; 2],
}

impl Vertex {
    fn lerp(&self, other: &Vertex, t: f32) -> Vertex {
        Vertex {
            pos: self.pos.lerp(other.pos, t),
            normal: self.normal.lerp(other.normal, t).normalize(),
            uv: [
                self.uv[0] + (other.uv[0] - self.uv[0]) * t,
                self.uv[1] + (other.uv[1] - self.uv[1]) * t,
            ],
        }
    }
}

#[derive(Clone, Debug)]
struct Polygon {
    vertices: Vec<Vertex>,
    plane: Plane,
}

impl Polygon {
    fn from_vertices(vertices: Vec<Vertex>) -> Option<Self> {
        if vertices.len() < 3 {
            return None;
        }
        let plane = Plane::from_points(vertices[0].pos, vertices[1].pos, vertices[2].pos)?;
        Some(Polygon { vertices, plane })
    }

    fn flip(&mut self) {
        self.vertices.reverse();
        for v in &mut self.vertices {
            v.normal = -v.normal;
        }
        self.plane.flip();
    }
}

#[derive(Clone, Copy, Debug)]
struct Plane {
    normal: Vec3,
    w: f32,
}

impl Plane {
    fn from_points(a: Vec3, b: Vec3, c: Vec3) -> Option<Self> {
        let normal = (b - a).cross(c - a).normalize();
        if normal.is_nan() || normal.length_squared() < 0.5 {
            return None;
        }
        Some(Plane { normal, w: normal.dot(a) })
    }

    fn flip(&mut self) {
        self.normal = -self.normal;
        self.w = -self.w;
    }
}

#[derive(Clone, Copy, PartialEq)]
enum PointClass {
    Coplanar,
    Front,
    Back,
}

fn classify_point(plane: &Plane, point: Vec3) -> PointClass {
    let t = plane.normal.dot(point) - plane.w;
    if t > EPSILON { PointClass::Front }
    else if t < -EPSILON { PointClass::Back }
    else { PointClass::Coplanar }
}

/// Split a polygon by a plane into front/back/coplanar-front/coplanar-back buckets.
fn split_polygon(
    plane: &Plane,
    polygon: &Polygon,
    coplanar_front: &mut Vec<Polygon>,
    coplanar_back: &mut Vec<Polygon>,
    front: &mut Vec<Polygon>,
    back: &mut Vec<Polygon>,
) {
    let mut classes = Vec::with_capacity(polygon.vertices.len());
    let mut has_front = false;
    let mut has_back = false;

    for v in &polygon.vertices {
        let c = classify_point(plane, v.pos);
        if c == PointClass::Front { has_front = true; }
        if c == PointClass::Back { has_back = true; }
        classes.push(c);
    }

    if !has_front && !has_back {
        // All coplanar
        if plane.normal.dot(polygon.plane.normal) > 0.0 {
            coplanar_front.push(polygon.clone());
        } else {
            coplanar_back.push(polygon.clone());
        }
        return;
    }

    if !has_back {
        front.push(polygon.clone());
        return;
    }

    if !has_front {
        back.push(polygon.clone());
        return;
    }

    // Spanning — split the polygon
    let mut f_verts = Vec::new();
    let mut b_verts = Vec::new();
    let n = polygon.vertices.len();

    for i in 0..n {
        let j = (i + 1) % n;
        let ci = classes[i];
        let cj = classes[j];
        let vi = &polygon.vertices[i];
        let vj = &polygon.vertices[j];

        if ci != PointClass::Back {
            f_verts.push(vi.clone());
        }
        if ci != PointClass::Front {
            b_verts.push(vi.clone());
        }

        if (ci == PointClass::Front && cj == PointClass::Back)
            || (ci == PointClass::Back && cj == PointClass::Front)
        {
            let t = (plane.w - plane.normal.dot(vi.pos))
                / plane.normal.dot(vj.pos - vi.pos);
            let t = t.clamp(0.0, 1.0);
            let mid = vi.lerp(vj, t);
            f_verts.push(mid.clone());
            b_verts.push(mid);
        }
    }

    if f_verts.len() >= 3 {
        if let Some(p) = Polygon::from_vertices(f_verts) {
            front.push(p);
        }
    }
    if b_verts.len() >= 3 {
        if let Some(p) = Polygon::from_vertices(b_verts) {
            back.push(p);
        }
    }
}

// =====================================================================
// BSP Node
// =====================================================================

struct BspNode {
    plane: Option<Plane>,
    front: Option<Box<BspNode>>,
    back: Option<Box<BspNode>>,
    polygons: Vec<Polygon>,
}

impl BspNode {
    fn new() -> Self {
        BspNode { plane: None, front: None, back: None, polygons: Vec::new() }
    }

    fn from_polygons(polygons: Vec<Polygon>) -> Self {
        let mut node = BspNode::new();
        if !polygons.is_empty() {
            node.build(polygons);
        }
        node
    }

    fn invert(&mut self) {
        for poly in &mut self.polygons {
            poly.flip();
        }
        if let Some(ref mut plane) = self.plane {
            plane.flip();
        }
        if let Some(ref mut front) = self.front {
            front.invert();
        }
        if let Some(ref mut back) = self.back {
            back.invert();
        }
        std::mem::swap(&mut self.front, &mut self.back);
    }

    /// Return all polygons in front of this BSP tree (clips away back polygons).
    fn clip_polygons(&self, polygons: &[Polygon]) -> Vec<Polygon> {
        let Some(ref plane) = self.plane else {
            return polygons.to_vec();
        };

        let mut front_list = Vec::new();
        let mut back_list = Vec::new();

        for poly in polygons {
            let mut cf = Vec::new();
            let mut cb = Vec::new();
            let mut f = Vec::new();
            let mut b = Vec::new();
            split_polygon(plane, poly, &mut cf, &mut cb, &mut f, &mut b);
            // Coplanar-front and front both go to front_list
            front_list.extend(cf);
            front_list.extend(f);
            // Coplanar-back and back both go to back_list
            back_list.extend(cb);
            back_list.extend(b);
        }

        if let Some(ref front) = self.front {
            front_list = front.clip_polygons(&front_list);
        }
        if let Some(ref back) = self.back {
            back_list = back.clip_polygons(&back_list);
        } else {
            back_list.clear();
        }

        front_list.extend(back_list);
        front_list
    }

    /// Remove all polygons in this tree that are inside the other tree.
    fn clip_to(&mut self, other: &BspNode) {
        self.polygons = other.clip_polygons(&self.polygons);
        if let Some(ref mut front) = self.front {
            front.clip_to(other);
        }
        if let Some(ref mut back) = self.back {
            back.clip_to(other);
        }
    }

    fn all_polygons(&self) -> Vec<Polygon> {
        let mut result = self.polygons.clone();
        if let Some(ref front) = self.front {
            result.extend(front.all_polygons());
        }
        if let Some(ref back) = self.back {
            result.extend(back.all_polygons());
        }
        result
    }

    fn build(&mut self, polygons: Vec<Polygon>) {
        if polygons.is_empty() {
            return;
        }

        if self.plane.is_none() {
            self.plane = Some(polygons[0].plane);
        }
        let plane = self.plane.unwrap();

        let mut coplanar = Vec::new();
        let mut front_list = Vec::new();
        let mut back_list = Vec::new();

        for poly in polygons {
            let mut cf = Vec::new();
            let mut cb = Vec::new();
            let mut f = Vec::new();
            let mut b = Vec::new();
            split_polygon(&plane, &poly, &mut cf, &mut cb, &mut f, &mut b);
            coplanar.extend(cf);
            coplanar.extend(cb);
            front_list.extend(f);
            back_list.extend(b);
        }
        self.polygons.extend(coplanar);

        if !front_list.is_empty() {
            if self.front.is_none() {
                self.front = Some(Box::new(BspNode::new()));
            }
            self.front.as_mut().unwrap().build(front_list);
        }
        if !back_list.is_empty() {
            if self.back.is_none() {
                self.back = Some(Box::new(BspNode::new()));
            }
            self.back.as_mut().unwrap().build(back_list);
        }
    }
}

// =====================================================================
// Boolean operations on BSP trees
// =====================================================================

fn bsp_union(a: BspNode, b: BspNode) -> BspNode {
    let mut a = a;
    let mut b = b;
    a.clip_to(&b);
    b.clip_to(&a);
    b.invert();
    b.clip_to(&a);
    b.invert();
    let mut polys = a.all_polygons();
    polys.extend(b.all_polygons());
    BspNode::from_polygons(polys)
}

fn bsp_subtract(a: BspNode, b: BspNode) -> BspNode {
    let mut a = a;
    let mut b = b;
    a.invert();
    a.clip_to(&b);
    b.clip_to(&a);
    b.invert();
    b.clip_to(&a);
    b.invert();
    let mut polys = a.all_polygons();
    polys.extend(b.all_polygons());
    let mut result = BspNode::from_polygons(polys);
    result.invert();
    result
}

fn bsp_intersect(a: BspNode, b: BspNode) -> BspNode {
    let mut a = a;
    let mut b = b;
    a.invert();
    b.clip_to(&a);
    b.invert();
    a.clip_to(&b);
    b.clip_to(&a);
    let mut polys = a.all_polygons();
    polys.extend(b.all_polygons());
    let mut result = BspNode::from_polygons(polys);
    result.invert();
    result
}

// =====================================================================
// Conversion between RawMesh and BSP polygons
// =====================================================================

fn mesh_to_bsp(mesh: RawMesh) -> BspNode {
    let mut polygons = Vec::new();

    for tri in mesh.indices.chunks(3) {
        if tri.len() < 3 { continue; }
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);

        let verts = vec![
            Vertex {
                pos: Vec3::from(mesh.positions[i0]),
                normal: Vec3::from(mesh.normals[i0]),
                uv: mesh.uvs[i0],
            },
            Vertex {
                pos: Vec3::from(mesh.positions[i1]),
                normal: Vec3::from(mesh.normals[i1]),
                uv: mesh.uvs[i1],
            },
            Vertex {
                pos: Vec3::from(mesh.positions[i2]),
                normal: Vec3::from(mesh.normals[i2]),
                uv: mesh.uvs[i2],
            },
        ];

        if let Some(mut poly) = Polygon::from_vertices(verts) {
            // Ensure BSP plane normal agrees with the vertex normals.
            // Vertex normals are authoritative (always outward-facing),
            // but winding order may not match across all primitive builders.
            let avg_normal = (Vec3::from(mesh.normals[i0])
                + Vec3::from(mesh.normals[i1])
                + Vec3::from(mesh.normals[i2]))
                .normalize();
            if poly.plane.normal.dot(avg_normal) < 0.0 {
                poly.flip();
            }
            polygons.push(poly);
        }
    }

    BspNode::from_polygons(polygons)
}

fn bsp_to_mesh(bsp: BspNode) -> RawMesh {
    let polygons = bsp.all_polygons();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    for poly in &polygons {
        if poly.vertices.len() < 3 { continue; }

        let base = positions.len() as u32;

        for v in &poly.vertices {
            positions.push([v.pos.x, v.pos.y, v.pos.z]);
            normals.push([v.normal.x, v.normal.y, v.normal.z]);
            uvs.push(v.uv);
        }

        // Fan triangulation for convex polygon
        for i in 1..(poly.vertices.len() as u32 - 1) {
            indices.push(base);
            indices.push(base + i);
            indices.push(base + i + 1);
        }
    }

    RawMesh { positions, normals, uvs, indices }
}
