//! Spherical ball surfaces with presentation-only rolling orientation.
use bevy::{
    asset::{load_internal_asset, uuid_handle},
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
    sprite_render::{AlphaMode2d, Material2d, Material2dPlugin},
};

use crate::{BALL_RADIUS, CanonicalBall};

const SHADER: Handle<Shader> = uuid_handle!("294e8038-a855-4478-b6b9-a4132b05c18e");

pub struct BallArtPlugin;

impl Plugin for BallArtPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, SHADER, "ball_art.wgsl", Shader::from_wgsl);
        app.add_plugins(Material2dPlugin::<BallMaterial>::default());
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct BallMaterial {
    #[uniform(0)]
    orientation: Vec4,
    #[uniform(0)]
    number: Vec4,
}

impl Material2d for BallMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

#[derive(Component)]
pub struct RollingSurface {
    previous: Vec2,
    orientation: Quat,
    hidden: bool,
}

pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<BallMaterial>,
    presentation: &crate::CanonicalPresentation,
) {
    let mesh = meshes.add(Rectangle::from_size(Vec2::splat(BALL_RADIUS * 2.5)));
    for number in 0..=15 {
        let position = presentation.target[&number];
        commands.spawn((
            Mesh2d(mesh.clone()),
            MeshMaterial2d(materials.add(BallMaterial {
                orientation: Quat::IDENTITY.to_array().into(),
                number: Vec4::new(f32::from(number), 0.0, 0.0, 0.0),
            })),
            Transform::from_translation(position.extend(4.0)),
            CanonicalBall(number),
            RollingSurface {
                previous: position,
                orientation: Quat::IDENTITY,
                hidden: false,
            },
        ));
    }
}

fn roll(orientation: Quat, displacement: Vec2) -> Quat {
    let distance = displacement.length();
    if distance <= f32::EPSILON {
        return orientation;
    }
    let axis = Vec3::new(-displacement.y, displacement.x, 0.0) / distance;
    (Quat::from_axis_angle(axis, distance / BALL_RADIUS) * orientation).normalize()
}

#[allow(clippy::needless_pass_by_value, clippy::type_complexity)]
pub fn update(
    mut balls: Query<(
        &Transform,
        &Visibility,
        &MeshMaterial2d<BallMaterial>,
        &mut RollingSurface,
    )>,
    mut materials: ResMut<Assets<BallMaterial>>,
) {
    for (transform, visibility, material, mut surface) in &mut balls {
        let position = transform.translation.truncate();
        let hidden = *visibility == Visibility::Hidden;
        if !hidden && !surface.hidden {
            surface.orientation = roll(surface.orientation, position - surface.previous);
        }
        surface.previous = position;
        surface.hidden = hidden;
        if let Some(mut material) = materials.get_mut(&material.0) {
            material.orientation = surface.orientation.to_array().into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_distance_and_direction_follow_sphere_geometry() {
        let quarter = BALL_RADIUS * std::f32::consts::FRAC_PI_2;
        let right = roll(Quat::IDENTITY, Vec2::new(quarter, 0.0));
        assert!((right * Vec3::Z - Vec3::X).length() < 0.0001);
        let up = roll(Quat::IDENTITY, Vec2::new(0.0, quarter));
        assert!((up * Vec3::Z - Vec3::Y).length() < 0.0001);
        let reversed = roll(right, Vec2::new(-quarter, 0.0));
        assert!((reversed * Vec3::Z - Vec3::Z).length() < 0.0001);
    }

    #[test]
    fn rolling_is_frame_partition_independent_and_stationary_balls_keep_orientation() {
        let full = roll(Quat::IDENTITY, Vec2::new(40.0, 20.0));
        let mut split = Quat::IDENTITY;
        for _ in 0..10 {
            split = roll(split, Vec2::new(4.0, 2.0));
        }
        assert!((full * Vec3::Z - split * Vec3::Z).length() < 0.0001);
        assert_eq!(roll(split, Vec2::ZERO), split);
    }
}
