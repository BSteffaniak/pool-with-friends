//! Original tapered cue artwork, baked once into a small translucent texture.
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

/// Cue length in table presentation coordinates, unaffected by edge clearance.
pub const LENGTH: f32 = 420.0;

const WIDTH: u32 = 1024;
const HEIGHT: u32 = 48;

pub fn spawn(commands: &mut Commands, images: &mut Assets<Image>) {
    let image = images.add(Image::new(
        Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    ));
    commands.spawn((
        Sprite {
            image,
            custom_size: Some(Vec2::new(LENGTH, 22.0)),
            ..default()
        },
        Transform::from_xyz(-552.0, 0.0, 7.0),
        crate::Cue,
        crate::MatchControlChrome,
    ));
}

// x runs from the butt at the left to the tip at the right. The transparent
// vertical margin preserves the existing cue length and tip alignment.
fn radius(x: f32) -> f32 {
    if x < 0.018 {
        0.62
    } else {
        (1.0 - x).mul_add(0.45, 0.22)
    }
}

#[allow(clippy::suboptimal_flops)] // Readable artwork equations, evaluated only at startup.
fn surface(x: f32, y: f32) -> [f32; 3] {
    if x < 0.018 {
        return [0.055, 0.065, 0.06];
    } // Rubber bumper.
    if x > 0.989 {
        return [0.20, 0.54, 0.61];
    } // Chalk-blue leather tip.
    if x > 0.959 {
        return [0.93, 0.91, 0.82];
    } // Ferrule.
    if [0.055, 0.29, 0.48]
        .iter()
        .any(|ring| (x - ring).abs() < 0.004)
    {
        return [0.76, 0.64, 0.37];
    }
    if (0.065..0.29).contains(&x) {
        let weave = ((x * 900.0 + y * 9.0).sin() * 0.5 + 0.5) * 0.045;
        return [0.055 + weave, 0.075 + weave, 0.065 + weave];
    }
    let grain = (x * 95.0 + y * 5.0).sin() * 0.018 + (x * 320.0 + y * 3.0).sin() * 0.009;
    if x < 0.48 {
        // Long maple points set into a dark walnut forearm.
        let point = x > 0.31 && y.abs() < (0.48 - x) * 3.0;
        if !point {
            return [0.22 + grain, 0.095 + grain, 0.045 + grain];
        }
    }
    [0.86 + grain, 0.65 + grain, 0.38 + grain]
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn pixels() -> Vec<u8> {
    let mut data = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for row in 0..HEIGHT {
        for column in 0..WIDTH {
            let x = (column as f32 + 0.5) / WIDTH as f32;
            let y = ((row as f32 + 0.5) / HEIGHT as f32).mul_add(2.0, -1.0);
            let cross = y / radius(x);
            let rgba = if cross.abs() < 1.0 {
                let normal_z = (1.0 - cross * cross).sqrt();
                let diffuse = (-0.45_f32).mul_add(cross, 0.82 * normal_z).max(0.0);
                let gloss = (-((cross + 0.35) / 0.16).powi(2)).exp() * 0.18;
                let color = surface(x, cross).map(|channel| {
                    channel
                        .mul_add(0.60_f32.mul_add(diffuse, 0.40), gloss)
                        .clamp(0.0, 1.0)
                });
                [
                    color[0],
                    color[1],
                    color[2],
                    ((1.0 - cross.abs()) * 14.0).min(1.0),
                ]
            } else {
                [0.0; 4]
            };
            data.extend(rgba.map(|value| (value * 255.0).round() as u8));
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cue_tapers_toward_tip_and_has_transparent_margins() {
        assert!(radius(0.1) > radius(0.9));
        let data = pixels();
        assert_eq!(data.len(), (WIDTH * HEIGHT * 4) as usize);
        assert!(
            data[..(WIDTH * 4) as usize]
                .chunks_exact(4)
                .all(|pixel| pixel[3] == 0)
        );
        let center = ((HEIGHT / 2 * WIDTH + WIDTH - 2) * 4) as usize;
        assert!(data[center + 2] > data[center]); // Blue tip, on the forward end.
        assert!(data[center + 3] > 200);
    }
}
