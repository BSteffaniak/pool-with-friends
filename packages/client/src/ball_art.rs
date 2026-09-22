//! Original, procedurally shaded ball artwork shared by all client modes.
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

use crate::{BALL_RADIUS, CanonicalBall};

const SIZE: u32 = 96;
const EXTENT: f32 = 1.25;
const COLORS: [[f32; 3]; 8] = [
    [0.96, 0.75, 0.08],
    [0.10, 0.30, 0.85],
    [0.88, 0.12, 0.10],
    [0.40, 0.12, 0.58],
    [0.95, 0.38, 0.04],
    [0.08, 0.47, 0.20],
    [0.50, 0.12, 0.08],
    [0.04, 0.04, 0.04],
];

pub fn spawn(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    presentation: &crate::CanonicalPresentation,
) {
    for number in 0..=15 {
        let image = images.add(Image::new(
            Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            pixels(number),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        ));
        let mut entity = commands.spawn((
            Sprite {
                image,
                custom_size: Some(Vec2::splat(BALL_RADIUS * 2.0 * EXTENT)),
                ..default()
            },
            Transform::from_translation(presentation.target[&number].extend(4.0)),
            CanonicalBall(number),
        ));
        if number != 0 {
            entity.with_children(|parent| {
                parent.spawn((
                    Text2d::new(number.to_string()),
                    TextFont::from_font_size(10.0),
                    TextColor(Color::srgb(0.035, 0.045, 0.055)),
                    Transform::from_xyz(0.0, 0.0, 0.1),
                ));
            });
        }
    }
}

fn surface(number: u8, x: f32, y: f32) -> [f32; 3] {
    if number == 0 || y.mul_add(y, x * x) < 0.43 * 0.43 || (number > 8 && y.abs() > 0.48) {
        [0.97, 0.96, 0.91]
    } else {
        COLORS[usize::from((number - 1) % 8)]
    }
}

// Keep the lighting equations readable; this runs once to bake tiny textures.
#[allow(clippy::suboptimal_flops)]
fn pixels(number: u8) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for row in 0..SIZE {
        for column in 0..SIZE {
            #[allow(clippy::cast_precision_loss)]
            let (x, y) = (
                ((column as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0) * EXTENT,
                ((row as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0) * EXTENT,
            );
            let radius_squared = x * x + y * y;
            let rgba = if radius_squared < 1.0 {
                let z = (1.0 - radius_squared).sqrt();
                let light = (-0.35 * x - 0.45 * y + 0.82 * z).max(0.0);
                let highlight = ((x + 0.30).powi(2) + (y + 0.38).powi(2)) / 0.025;
                let gloss = (-highlight).exp() * 0.50;
                let color = surface(number, x, y)
                    .map(|channel| (channel * (0.42 + 0.58 * light) + gloss).min(1.0));
                [
                    color[0],
                    color[1],
                    color[2],
                    ((1.0 - radius_squared.sqrt()) * 40.0).min(1.0),
                ]
            } else {
                let shadow = (x - 0.07).hypot(y - 0.12);
                [0.0, 0.0, 0.0, ((1.17 - shadow) * 2.0).clamp(0.0, 0.28)]
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            pixels.extend(rgba.map(|channel| (channel * 255.0).round() as u8));
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::float_cmp)] // Compares exact palette entries, not computed lighting.
    fn stripes_have_ivory_caps_and_matching_solid_colors() {
        for number in 1..=7 {
            assert_eq!(surface(number, 0.6, 0.0), surface(number + 8, 0.6, 0.0));
            assert_ne!(surface(number, 0.0, 0.8), surface(number + 8, 0.0, 0.8));
        }
        assert_eq!(surface(8, 0.6, 0.0), COLORS[7]);
        assert_eq!(surface(0, 0.6, 0.0), surface(15, 0.0, 0.0));
    }

    #[test]
    fn textures_are_shaded_with_transparent_corners() {
        for number in 0..=15 {
            let data = pixels(number);
            assert_eq!(data.len(), (SIZE * SIZE * 4) as usize);
            assert_eq!(data[3], 0);
            let upper = ((30 * SIZE + 30) * 4) as usize;
            let lower = ((65 * SIZE + 65) * 4) as usize;
            assert_ne!(&data[upper..upper + 3], &data[lower..lower + 3]);
        }
    }
}
