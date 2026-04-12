use bevy::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};
use super::meshes::RawMesh;

// =====================================================================
// Stats tracking
// =====================================================================

/// Tracks the maximum clip_polygons recursion depth across all CSG operations.
static MAX_CLIP_DEPTH: AtomicU32 = AtomicU32::new(0);

fn track_clip_depth(depth: u32) {
    MAX_CLIP_DEPTH.fetch_max(depth, Ordering::Relaxed);
}

/// Statistics from a single CSG pipeline run.
#[derive(Debug, Clone, Default)]
pub struct CsgStats {
    pub input_union_tris: Vec<u32>,
    pub input_subtract_tris: Vec<u32>,
    pub input_clip_tris: Vec<u32>,
    pub max_bsp_depth: u32,
    pub max_bsp_polys: u32,
    pub max_clip_recursion: u32,
    pub output_tris: u32,
}

fn reset_clip_depth() {
    MAX_CLIP_DEPTH.store(0, Ordering::Relaxed);
}

fn read_clip_depth() -> u32 {
    MAX_CLIP_DEPTH.load(Ordering::Relaxed)
}

fn bsp_depth(node: &BspNode) -> u32 {
    let mut max = 0u32;
    let mut stack: Vec<(&BspNode, u32)> = vec![(node, 1)];
    while let Some((n, d)) = stack.pop() {
        max = max.max(d);
        if let Some(ref front) = n.front { stack.push((front, d + 1)); }
        if let Some(ref back) = n.back { stack.push((back, d + 1)); }
    }
    max
}

fn bsp_polygon_count(node: &BspNode) -> u32 {
    let mut count = 0u32;
    let mut stack: Vec<&BspNode> = vec![node];
    while let Some(n) = stack.pop() {
        count += n.polygons.len() as u32;
        if let Some(ref front) = n.front { stack.push(front); }
        if let Some(ref back) = n.back { stack.push(back); }
    }
    count
}

// =====================================================================
// Public API
// =====================================================================

/// Perform the full CSG pipeline, returning both the result mesh and stats.
pub fn perform_csg_pipeline(
    union_meshes: Vec<RawMesh>,
    subtract_meshes: Vec<RawMesh>,
    clip_meshes: Vec<RawMesh>,
) -> (RawMesh, CsgStats) {
    let mut stats = CsgStats::default();

    if union_meshes.is_empty() {
        return (RawMesh { positions: vec![], normals: vec![], uvs: vec![], indices: vec![] }, stats);
    }

    reset_clip_depth();

    for m in &union_meshes { stats.input_union_tris.push(m.indices.len() as u32 / 3); }
    for m in &subtract_meshes { stats.input_subtract_tris.push(m.indices.len() as u32 / 3); }
    for m in &clip_meshes { stats.input_clip_tris.push(m.indices.len() as u32 / 3); }

    let mut iter = union_meshes.into_iter();
    let mut result = mesh_to_bsp(iter.next().unwrap());
    stats.max_bsp_depth = stats.max_bsp_depth.max(bsp_depth(&result));
    stats.max_bsp_polys = stats.max_bsp_polys.max(bsp_polygon_count(&result));

    for mesh in iter {
        let operand = mesh_to_bsp(mesh);
        stats.max_bsp_depth = stats.max_bsp_depth.max(bsp_depth(&operand));
        stats.max_bsp_polys = stats.max_bsp_polys.max(bsp_polygon_count(&operand));
        result = bsp_union(result, operand);
        stats.max_bsp_depth = stats.max_bsp_depth.max(bsp_depth(&result));
        stats.max_bsp_polys = stats.max_bsp_polys.max(bsp_polygon_count(&result));
    }

    for mesh in subtract_meshes {
        let operand = mesh_to_bsp(mesh);
        stats.max_bsp_depth = stats.max_bsp_depth.max(bsp_depth(&operand));
        result = bsp_subtract(result, operand);
        stats.max_bsp_depth = stats.max_bsp_depth.max(bsp_depth(&result));
        stats.max_bsp_polys = stats.max_bsp_polys.max(bsp_polygon_count(&result));
    }

    for mesh in clip_meshes {
        let operand = mesh_to_bsp(mesh);
        stats.max_bsp_depth = stats.max_bsp_depth.max(bsp_depth(&operand));
        result = bsp_intersect(result, operand);
        stats.max_bsp_depth = stats.max_bsp_depth.max(bsp_depth(&result));
        stats.max_bsp_polys = stats.max_bsp_polys.max(bsp_polygon_count(&result));
    }

    stats.max_clip_recursion = read_clip_depth();

    let out = bsp_to_mesh(result);
    stats.output_tris = out.indices.len() as u32 / 3;
    (out, stats)
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

    fn clip_polygons(&self, polygons: &[Polygon]) -> Vec<Polygon> {
        self.clip_polygons_depth(polygons, 0)
    }

    fn clip_polygons_depth(&self, polygons: &[Polygon], depth: u32) -> Vec<Polygon> {
        track_clip_depth(depth);

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
            front_list.extend(cf);
            front_list.extend(f);
            back_list.extend(cb);
            back_list.extend(b);
        }

        if let Some(ref front) = self.front {
            front_list = front.clip_polygons_depth(&front_list, depth + 1);
        }
        if let Some(ref back) = self.back {
            back_list = back.clip_polygons_depth(&back_list, depth + 1);
        } else {
            back_list.clear();
        }

        front_list.extend(back_list);
        front_list
    }

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
// Boolean operations
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
        let face_normal = [poly.plane.normal.x, poly.plane.normal.y, poly.plane.normal.z];

        for v in &poly.vertices {
            positions.push([v.pos.x, v.pos.y, v.pos.z]);
            normals.push(face_normal);
            uvs.push(v.uv);
        }

        for i in 1..(poly.vertices.len() as u32 - 1) {
            indices.push(base);
            indices.push(base + i);
            indices.push(base + i + 1);
        }
    }

    RawMesh { positions, normals, uvs, indices }
}
