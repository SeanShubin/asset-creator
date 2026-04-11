use bevy::prelude::*;
use crate::registry::AssetRegistry;
use crate::util::Color3;
use super::animation::ShapeAnimator;
use super::csg;
use super::definition::{Axis, Bounds, Combinator, CombineMode, PrimitiveShape, RepeatSpec, ShapeNode, reflect_orient};

// =====================================================================
// Components
// =====================================================================

#[derive(Component, Clone, Debug)]
pub struct ShapePart {
    pub name: Option<String>,
}

#[derive(Component, Clone, Debug)]
pub struct BaseTransform(pub Transform);

#[derive(Component)]
pub struct ShapeRoot;

/// Marks an entity whose children participate in CSG.
/// Stores the data needed to rebuild the CSG mesh when children are toggled.
#[derive(Component, Clone)]
pub struct CsgGroup {
    pub children: Vec<ShapeNode>,
    pub colors: ColorMap,
}

/// Marks the entity that holds the CSG result mesh.
#[derive(Component)]
pub struct CsgResult;

/// Tracks which CSG children were active last rebuild, so we detect changes.
#[derive(Component)]
pub struct CsgChildState {
    pub active: Vec<bool>,
}

/// Marks a ShapePart entity whose rendered geometry is suppressed because
/// it participates in a CSG group. The part is visible in the tree but its
/// own mesh is hidden — the CsgResult mesh renders instead.
#[derive(Component)]
pub struct CsgMember;

// =====================================================================
// Public API
// =====================================================================

pub fn spawn_shape(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    shape: &ShapeNode,
    registry: &AssetRegistry,
) -> Entity {
    validate_names(shape, "");

    let position = bounds_center(&shape.bounds);
    let root_tf = Transform::from_translation(position);
    let root = commands.spawn((
        ShapeRoot,
        ShapePart { name: shape.name.clone() },
        BaseTransform(root_tf),
        ShapeAnimator::new(shape.animations.clone()),
        root_tf,
        Visibility::default(),
    )).id();

    let colors = shape.palette.clone();
    process_node(commands, meshes, materials, root, shape, &colors, registry);
    root
}

fn validate_names(node: &ShapeNode, path: &str) {
    let node_path = match &node.name {
        Some(name) => {
            if path.is_empty() { name.clone() } else { format!("{path}/{name}") }
        }
        None => {
            if node.shape.is_some() {
                warn!("Unnamed shape at path '{path}' — every shape should have a name");
            }
            path.to_string()
        }
    };

    // Check for duplicate names among children
    let mut seen = std::collections::HashSet::new();
    for child in &node.children {
        if let Some(ref name) = child.name {
            if !seen.insert(name.clone()) {
                warn!("Duplicate child name '{}' at path '{}'", name, node_path);
            }
        }
    }

    for child in &node.children {
        validate_names(child, &node_path);
    }
}

pub fn despawn_shape(commands: &mut Commands, roots: &[Entity]) {
    for &e in roots {
        commands.entity(e).despawn_recursive();
    }
}

// =====================================================================
// Color context
// =====================================================================

type ColorMap = Vec<(String, Color3)>;

/// Merge parent colors over child colors. Parent wins on conflict.
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

/// Apply color_map or colors from an import node to the imported shape's palette.
/// Returns the remapped color map using the parent's color context.
fn apply_color_remapping(
    import_node: &ShapeNode,
    imported_colors: &ColorMap,
    parent_colors: &ColorMap,
) -> ColorMap {
    if !import_node.color_map.is_empty() && !import_node.colors.is_empty() {
        warn!("Node '{}' specifies both color_map and colors — using color_map",
            import_node.name.as_deref().unwrap_or("unnamed"));
    }

    if !import_node.color_map.is_empty() {
        // Named remapping: child color name → parent color name
        imported_colors.iter().map(|(child_name, child_val)| {
            if let Some(parent_name) = import_node.color_map.get(child_name) {
                let resolved = resolve_color(parent_name, parent_colors);
                (child_name.clone(), resolved)
            } else {
                (child_name.clone(), *child_val)
            }
        }).collect()
    } else if !import_node.colors.is_empty() {
        // Positional remapping: parent color names in order
        imported_colors.iter().enumerate().map(|(i, (child_name, child_val))| {
            if let Some(parent_name) = import_node.colors.get(i) {
                let resolved = resolve_color(parent_name, parent_colors);
                (child_name.clone(), resolved)
            } else {
                (child_name.clone(), *child_val)
            }
        }).collect()
    } else {
        // No remapping — use parent colors merged over imported colors
        merge_colors(parent_colors, imported_colors)
    }
}

/// Resolve a color name to a Color3 value using the color context.
fn resolve_color(name: &str, colors: &ColorMap) -> Color3 {
    colors.iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| *v)
        .unwrap_or_else(|| {
            warn!("Color '{}' not found in color map, using default gray", name);
            Color3(0.5, 0.5, 0.5)
        })
}

// =====================================================================
// Node processing
// =====================================================================

fn process_node(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    node: &ShapeNode,
    colors: &ColorMap,
    registry: &AssetRegistry,
) {
    // Merge this node's color definitions into the context
    let colors = if node.palette.is_empty() {
        colors.clone()
    } else {
        merge_colors(colors, &node.palette)
    };

    match node.combinator() {
        Combinator::Mirror(axes) => {
            process_mirror(commands, meshes, materials, parent, node, axes, &colors, registry);
        }
        Combinator::Repeat(repeat) => {
            process_repeat(commands, meshes, materials, parent, node, repeat, &colors, registry);
        }
        Combinator::Import(import_name) => {
            process_import(commands, meshes, materials, parent, node, import_name, &colors, registry);
        }
        Combinator::None => {
            attach_geometry(commands, meshes, materials, parent, node, &colors);
            if node.has_csg_children() {
                process_csg_children(commands, meshes, materials, parent, node, &colors, registry);
            } else {
                for child in &node.children {
                    spawn_child(commands, meshes, materials, parent, child, &colors, registry);
                }
            }
        }
    }
}

// =====================================================================
// Import
// =====================================================================

fn process_import(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    node: &ShapeNode,
    import_name: &str,
    colors: &ColorMap,
    registry: &AssetRegistry,
) {
    let imported = match registry.get_shape(import_name) {
        Some(shape) => shape.clone(),
        None => {
            error!("Import '{}' not found in registry", import_name);
            return;
        }
    };

    let native_aabb = imported.compute_aabb()
        .unwrap_or(Bounds(-0.5, -0.5, -0.5, 0.5, 0.5, 0.5));
    let placement = node.bounds.unwrap_or(native_aabb);

    let mut remapped = imported;
    remapped.remap_bounds(&native_aabb, &placement);

    // Apply color remapping from the import node
    let import_colors = apply_color_remapping(node, &remapped.palette, colors);

    attach_geometry(commands, meshes, materials, parent, &remapped, &import_colors);
    for child in &remapped.children {
        spawn_child(commands, meshes, materials, parent, child, &import_colors, registry);
    }
}

// =====================================================================
// CSG — triggered when any child has combine: Subtract or Clip
// =====================================================================

fn process_csg_children(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    node: &ShapeNode,
    colors: &ColorMap,
    registry: &AssetRegistry,
) {
    // Spawn all children as normal entities (they appear in the part tree),
    // but mark them as CSG members so their individual meshes are suppressed.
    for child in &node.children {
        let child_entity = spawn_child_entity(commands, meshes, materials, parent, child, colors, registry);
        commands.entity(child_entity).insert(CsgMember);
    }

    // Store CSG data on the parent for rebuild on toggle
    let all_active = vec![true; node.children.len()];
    commands.entity(parent).insert((
        CsgGroup {
            children: node.children.clone(),
            colors: colors.clone(),
        },
        CsgChildState { active: all_active },
    ));

    // Build and attach the CSG result mesh
    build_csg_mesh(commands, meshes, materials, parent, &node.children, colors, registry, node);
}

/// Build the CSG result from the given children and attach it to the parent.
fn build_csg_mesh(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    children: &[ShapeNode],
    colors: &ColorMap,
    registry: &AssetRegistry,
    node: &ShapeNode,
) {
    let identity = Transform::IDENTITY;
    let mut union_meshes = Vec::new();
    let mut subtract_meshes = Vec::new();
    let mut clip_meshes = Vec::new();

    for child in children {
        let raw = csg::collect_node_mesh(child, identity, colors, registry);
        if raw.positions.is_empty() { continue; }
        match child.combine {
            CombineMode::Union => union_meshes.push(raw),
            CombineMode::Subtract => subtract_meshes.push(raw),
            CombineMode::Clip => clip_meshes.push(raw),
        }
    }

    if union_meshes.is_empty() {
        return;
    }

    let result = csg::perform_csg_pipeline(union_meshes, subtract_meshes, clip_meshes);

    if result.positions.is_empty() {
        return;
    }

    let color = node.color.as_ref()
        .map(|name| resolve_color(name, colors))
        .unwrap_or_else(|| {
            children.iter()
                .find(|c| c.combine == CombineMode::Union)
                .and_then(|c| c.color.as_ref())
                .map(|name| resolve_color(name, colors))
                .unwrap_or(Color3(0.5, 0.5, 0.5))
        });

    let base_color = Color::srgb(color.0, color.1, color.2);
    let material = materials.add(StandardMaterial {
        base_color,
        cull_mode: None,
        ..default()
    });

    let mesh_handle = meshes.add(result.to_bevy_mesh());

    commands.entity(parent).with_child((
        CsgResult,
        Mesh3d(mesh_handle),
        MeshMaterial3d(material),
        Transform::IDENTITY,
    ));
}


// =====================================================================
// Combinators
// =====================================================================

fn process_repeat(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    node: &ShapeNode,
    repeat: &RepeatSpec,
    colors: &ColorMap,
    registry: &AssetRegistry,
) {
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
        if let Some(ref name) = instance.name {
            instance.name = Some(format!("{name}_{i}"));
        }
        spawn_child(commands, meshes, materials, parent, &instance, colors, registry);
    }
}

fn process_mirror(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    node: &ShapeNode,
    axes: &[Axis],
    colors: &ColorMap,
    registry: &AssetRegistry,
) {
    let mut base = node.clone();
    base.mirror = Vec::new();

    let combinations = mirror_combinations(axes);
    for (flipped_axes, suffix) in &combinations {
        let mut copy = base.clone();
        for &axis in flipped_axes {
            flip_node_bounds(&mut copy, axis);
        }
        for &axis in flipped_axes {
            reflect_orientation(&mut copy, axis);
        }
        if !suffix.is_empty() {
            if let Some(ref name) = copy.name {
                copy.name = Some(format!("{name}_{suffix}"));
            }
        }
        spawn_child(commands, meshes, materials, parent, &copy, colors, registry);
    }
}

/// Generate all 2^N combinations of flipping the given axes.
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
                let letter = match axis {
                    Axis::X => "x",
                    Axis::Y => "y",
                    Axis::Z => "z",
                };
                suffix.push_str(letter);
            }
        }
        let suffix = if suffix.is_empty() { String::new() } else { format!("m{suffix}") };
        result.push((flipped, suffix));
    }

    result
}

// =====================================================================
// Child spawning
// =====================================================================

fn spawn_child(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    node: &ShapeNode,
    colors: &ColorMap,
    registry: &AssetRegistry,
) {
    spawn_child_entity(commands, meshes, materials, parent, node, colors, registry);
}

fn spawn_child_entity(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    node: &ShapeNode,
    colors: &ColorMap,
    registry: &AssetRegistry,
) -> Entity {
    let child_tf = build_child_transform(node);
    let child = commands.spawn((
        ShapePart { name: node.name.clone() },
        BaseTransform(child_tf),
        child_tf,
        Visibility::default(),
    )).id();
    commands.entity(parent).add_child(child);

    process_node(commands, meshes, materials, child, node, colors, registry);
    child
}

fn build_child_transform(node: &ShapeNode) -> Transform {
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
    tf
}

// =====================================================================
// Geometry attachment
// =====================================================================

fn attach_geometry(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    node: &ShapeNode,
    colors: &ColorMap,
) {
    let Some(shape) = &node.shape else { return };
    let bounds = node.bounds.unwrap_or(Bounds(-0.5, -0.5, -0.5, 0.5, 0.5, 0.5));
    let om = node.orient;
    let is_mirrored = om.determinant() < 0.0;

    let color = node.color.as_ref()
        .map(|name| resolve_color(name, colors))
        .unwrap_or_else(|| {
            warn!("Shape '{}' has no color specified", node.name.as_deref().unwrap_or("unnamed"));
            Color3(0.5, 0.5, 0.5)
        });

    let (mesh, material) = make_mesh(meshes, materials, *shape, color, node.emissive, is_mirrored);
    let mesh_tf = mesh_transform(*shape, &bounds, &om);

    if node.children.is_empty() {
        commands.entity(parent).with_child((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            mesh_tf,
            Visibility::default(),
        ));
    } else {
        let shape_name = node.name.as_ref()
            .map(|n| format!("{n}_shape"))
            .unwrap_or_else(|| "shape".to_string());
        let shape_entity = commands.spawn((
            ShapePart { name: Some(shape_name) },
            BaseTransform(Transform::default()),
            Transform::default(),
            Visibility::default(),
        )).id();
        commands.entity(parent).add_child(shape_entity);
        commands.entity(shape_entity).with_child((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            mesh_tf,
            Visibility::default(),
        ));
    }
}

// =====================================================================
// Mesh creation
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

    let mat = bevy::math::Mat3::from_cols(col_x, col_y, col_z);
    let affine = bevy::math::Affine3A::from_mat3(mat);
    Transform::from_matrix(bevy::math::Mat4::from(affine))
}

fn pick_size_for_direction(dir: Vec3, size: (f32, f32, f32)) -> f32 {
    if dir.x.abs() > 0.5 { size.0 }
    else if dir.y.abs() > 0.5 { size.1 }
    else { size.2 }
}

fn make_mesh(
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    shape: PrimitiveShape,
    color: Color3,
    emissive: bool,
    is_mirrored: bool,
) -> (Handle<Mesh>, Handle<StandardMaterial>) {
    let mesh = match shape {
        PrimitiveShape::Box => meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        PrimitiveShape::Sphere => meshes.add(Sphere::new(0.5).mesh().build()),
        PrimitiveShape::Cylinder => meshes.add(Cylinder::new(0.5, 1.0).mesh().build()),
        PrimitiveShape::Cone => meshes.add(super::meshes::create_cone_mesh(24, 32)),
        PrimitiveShape::Dome => meshes.add(super::meshes::create_unit_dome(24, 32)),
        PrimitiveShape::Wedge => meshes.add(super::meshes::create_wedge_mesh()),
        PrimitiveShape::Torus => meshes.add(super::meshes::create_torus_mesh(32, 16)),
        PrimitiveShape::Corner => meshes.add(super::meshes::create_unit_corner()),
    };

    let base_color = Color::srgb(color.0, color.1, color.2);
    let cull_mode = if is_mirrored { None } else { Some(bevy::render::render_resource::Face::Back) };
    let material = if emissive {
        materials.add(StandardMaterial {
            base_color,
            emissive: base_color.into(),
            cull_mode,
            ..default()
        })
    } else {
        materials.add(StandardMaterial {
            base_color,
            cull_mode,
            ..default()
        })
    };

    (mesh, material)
}

// =====================================================================
// Helpers
// =====================================================================

const DEFAULT_BOUNDS: Bounds = Bounds(-0.5, -0.5, -0.5, 0.5, 0.5, 0.5);

fn reify_bounds(node: &mut ShapeNode) {
    if node.bounds.is_none() && node.shape.is_some() {
        node.bounds = Some(DEFAULT_BOUNDS);
    }
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

// =====================================================================
// CSG rebuild on visibility toggle
// =====================================================================

/// System that hides mesh geometry under CsgMember entities when a CsgResult
/// exists on their parent. When no CsgResult exists (all CSG ops toggled off),
/// the individual meshes are allowed to render normally.
pub fn suppress_csg_member_meshes(
    mut commands: Commands,
    members: Query<(Entity, &Parent), With<CsgMember>>,
    children_query: Query<&Children>,
    mesh_entities: Query<Entity, With<Mesh3d>>,
    csg_results: Query<&CsgResult>,
) {
    for (member, parent) in &members {
        // Check if parent has a CsgResult child
        let has_csg_result = children_query.get(parent.get())
            .map(|children| children.iter().any(|e| csg_results.get(*e).is_ok()))
            .unwrap_or(false);

        let target_vis = if has_csg_result { Visibility::Hidden } else { Visibility::Inherited };
        set_descendant_mesh_visibility(&mut commands, member, &children_query, &mesh_entities, target_vis);
    }
}

fn set_descendant_mesh_visibility(
    commands: &mut Commands,
    entity: Entity,
    children_query: &Query<&Children>,
    mesh_entities: &Query<Entity, With<Mesh3d>>,
    vis: Visibility,
) {
    if let Ok(children) = children_query.get(entity) {
        for &child in children.iter() {
            if mesh_entities.get(child).is_ok() {
                if let Some(mut ec) = commands.get_entity(child) {
                    ec.insert(vis);
                }
            }
            set_descendant_mesh_visibility(commands, child, children_query, mesh_entities, vis);
        }
    }
}

/// System that detects when CSG children are toggled and rebuilds the CSG mesh.
pub fn rebuild_csg_on_toggle(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    registry: Res<AssetRegistry>,
    mut csg_groups: Query<(Entity, &CsgGroup, &mut CsgChildState, &Children)>,
    parts: Query<&ShapePart>,
    visibility: Query<&Visibility>,
    csg_results: Query<Entity, With<CsgResult>>,
) {
    for (parent, group, mut state, children) in &mut csg_groups {
        // Determine which CSG children are currently active (not hidden).
        // CSG children are the ShapePart children of the parent, in order.
        let part_children: Vec<Entity> = children.iter()
            .filter(|e| parts.get(**e).is_ok())
            .copied()
            .collect();

        let current_active: Vec<bool> = part_children.iter()
            .map(|&e| {
                visibility.get(e)
                    .map(|v| *v != Visibility::Hidden)
                    .unwrap_or(true)
            })
            .collect();

        if current_active == state.active {
            continue;
        }
        state.active = current_active.clone();

        // Remove the old CSG result mesh
        for &child in children.iter() {
            if csg_results.get(child).is_ok() {
                if let Some(ec) = commands.get_entity(child) {
                    ec.despawn_recursive();
                }
            }
        }

        // Collect only the active children's ShapeNode data
        let active_children: Vec<ShapeNode> = group.children.iter()
            .zip(current_active.iter())
            .filter(|(_, active)| **active)
            .map(|(node, _)| node.clone())
            .collect();

        if active_children.is_empty() || !active_children.iter().any(|c| c.combine != CombineMode::Union) {
            // No CSG needed — children render themselves via normal visibility
            continue;
        }

        // Rebuild with a dummy parent node for color resolution
        let dummy_node = ShapeNode {
            name: None,
            shape: None,
            bounds: None,
            orient: Mat3::IDENTITY,
            palette: Vec::new(),
            color: group.children.first().and_then(|c| c.color.clone()),
            emissive: false,
            rotate: None,
            import: None,
            color_map: Default::default(),
            colors: Vec::new(),
            children: Vec::new(),
            mirror: Vec::new(),
            repeat: None,
            combine: CombineMode::Union,
            animations: Vec::new(),
        };

        build_csg_mesh(
            &mut commands, &mut meshes, &mut materials,
            parent, &active_children, &group.colors, &registry, &dummy_node,
        );
    }
}
