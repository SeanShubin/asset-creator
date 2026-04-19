use bevy::prelude::*;
use std::collections::HashMap;

use super::spec::{AnimProperty, AnimState, Axis, JointMotion};
use super::interpreter::{BaseTransform, ShapePart, ShapeRoot};

// =====================================================================
// Animator component
// =====================================================================

#[derive(Component, Clone, Debug)]
pub struct ShapeAnimator {
    pub states: Vec<AnimState>,
    pub active_state: Option<usize>,
    pub phase: f32,
    pub speed: f32,
    pub needs_reset: bool,
}

impl ShapeAnimator {
    pub fn new(states: Vec<AnimState>) -> Self {
        let active = if states.is_empty() { None } else { Some(0) };
        Self { states, active_state: active, phase: 0.0, speed: 1.0, needs_reset: false }
    }

    pub fn active_name(&self) -> &str {
        self.active_state
            .and_then(|i| self.states.get(i))
            .map(|s| s.name.as_str())
            .unwrap_or("(none)")
    }

    pub fn cycle_state(&mut self) {
        if self.states.is_empty() { return; }
        self.active_state = Some(match self.active_state {
            Some(i) => (i + 1) % self.states.len(),
            None => 0,
        });
        self.phase = 0.0;
        self.needs_reset = true;
    }
}

// =====================================================================
// Animation system
// =====================================================================

pub fn animate_shapes(
    time: Res<Time>,
    mut animators: Query<(&mut ShapeAnimator, &Children), With<ShapeRoot>>,
    parts: Query<(&ShapePart, Option<&Children>)>,
    base_transforms: Query<&BaseTransform>,
    mut transforms: Query<&mut Transform>,
) {
    for (mut animator, root_children) in &mut animators {
        animator.phase += time.delta_secs() * animator.speed;
        let phase = animator.phase;
        let t = time.elapsed_secs();

        let mut name_map: HashMap<String, Vec<Entity>> = HashMap::new();
        collect_named_parts(root_children, &parts, &mut name_map);

        if animator.needs_reset {
            animator.needs_reset = false;
            reset_to_base_transforms(&name_map, &base_transforms, &mut transforms);
        }

        let Some(state_idx) = animator.active_state else { continue };
        let Some(state) = animator.states.get(state_idx) else { continue };

        apply_animation_channels(state, &name_map, phase, t, &base_transforms, &mut transforms);
    }
}

// =====================================================================
// Animation helpers
// =====================================================================

fn apply_animation_channels(
    state: &AnimState,
    name_map: &HashMap<String, Vec<Entity>>,
    phase: f32,
    time: f32,
    base_transforms: &Query<&BaseTransform>,
    transforms: &mut Query<&mut Transform>,
) {
    for channel in &state.channels {
        let Some(entities) = name_map.get(&channel.part) else { continue };
        let value = evaluate_motion(&channel.motion, phase, time);

        for &entity in entities {
            let base = base_transforms.get(entity).map(|b| b.0).unwrap_or_default();
            let Ok(mut tf) = transforms.get_mut(entity) else { continue };
            apply_channel_value(&mut tf, &base, &channel.property, &channel.axis, value);
        }
    }
}

fn apply_channel_value(
    tf: &mut Transform,
    base: &Transform,
    property: &AnimProperty,
    axis: &Axis,
    value: f32,
) {
    match property {
        AnimProperty::Rotation => {
            let rot = match axis {
                Axis::X => Quat::from_rotation_x(value),
                Axis::Y => Quat::from_rotation_y(value),
                Axis::Z => Quat::from_rotation_z(value),
            };
            tf.rotation = base.rotation * rot;
        }
        AnimProperty::Translation => {
            match axis {
                Axis::X => tf.translation.x = base.translation.x + value,
                Axis::Y => tf.translation.y = base.translation.y + value,
                Axis::Z => tf.translation.z = base.translation.z + value,
            }
        }
    }
}

fn evaluate_motion(motion: &JointMotion, phase: f32, time: f32) -> f32 {
    match motion {
        JointMotion::Oscillate { amplitude, speed, offset } => {
            (phase * speed + offset).sin() * amplitude
        }
        JointMotion::Spin { rate } => phase * rate,
        JointMotion::Bob { amplitude, freq } => (time * freq).sin() * amplitude,
    }
}

fn reset_to_base_transforms(
    name_map: &HashMap<String, Vec<Entity>>,
    base_transforms: &Query<&BaseTransform>,
    transforms: &mut Query<&mut Transform>,
) {
    for entities in name_map.values() {
        for &entity in entities {
            if let Ok(base) = base_transforms.get(entity) {
                if let Ok(mut tf) = transforms.get_mut(entity) {
                    *tf = base.0;
                }
            }
        }
    }
}

fn collect_named_parts(
    children: &Children,
    parts: &Query<(&ShapePart, Option<&Children>)>,
    map: &mut HashMap<String, Vec<Entity>>,
) {
    for child in children.iter() {
        if let Ok((part, grandchildren)) = parts.get(child) {
            if let Some(ref name) = part.name {
                map.entry(name.clone()).or_default().push(child);
            }
            if let Some(gc) = grandchildren {
                collect_named_parts(gc, parts, map);
            }
        }
    }
}
