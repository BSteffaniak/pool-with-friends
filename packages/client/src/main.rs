#![cfg_attr(feature = "fail-on-warnings", deny(warnings))]
#![warn(clippy::all, clippy::pedantic, clippy::nursery, clippy::cargo)]
#![allow(clippy::multiple_crate_versions)]

use bevy::{
    camera::{OrthographicProjection, Projection, ScalingMode},
    color::palettes::css::{BLACK, WHITE},
    prelude::*,
    window::{PresentMode, WindowResolution},
};

const DESIGN_SIZE: Vec2 = Vec2::new(1280.0, 720.0);
const TABLE_CENTER_X: f32 = -55.0;
const TABLE_SIZE: Vec2 = Vec2::new(960.0, 480.0);
const CUSHION: f32 = 34.0;
const BALL_RADIUS: f32 = 13.0;
const POWER_BAR_HEIGHT: f32 = 300.0;
const POWER_ZONE_START: f32 = 0.82;
const MIN_POWER: f32 = 0.05;
const DEFAULT_POWER: f32 = 0.55;

#[derive(Component)]
struct Cue;

#[derive(Component)]
struct AimGuide;

#[derive(Component)]
struct PowerFill;

#[derive(Component)]
struct OrientationNotice;

#[derive(Resource)]
struct PrototypeInput {
    aim_angle: f32,
    power: f32,
    active_touch: Option<u64>,
}

impl Default for PrototypeInput {
    fn default() -> Self {
        Self {
            aim_angle: 0.25,
            power: DEFAULT_POWER,
            active_touch: None,
        }
    }
}

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.025, 0.075, 0.055)))
        .init_resource::<PrototypeInput>()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Pool with More Than Friends".into(),
                name: Some("pwmtf.client".into()),
                canvas: Some("#pwmtf-canvas".into()),
                resolution: WindowResolution::new(1280, 720),
                present_mode: PresentMode::AutoVsync,
                fit_canvas_to_parent: true,
                prevent_default_event_handling: true,
                ..default()
            }),
            ..default()
        }))
        .add_systems(Startup, setup)
        .add_systems(Update, (update_input, update_aim, update_orientation))
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::AutoMin {
                min_width: DESIGN_SIZE.x,
                min_height: DESIGN_SIZE.y,
            },
            ..OrthographicProjection::default_2d()
        }),
    ));

    spawn_rectangle(
        &mut commands,
        Vec2::new(
            CUSHION.mul_add(2.0, TABLE_SIZE.x),
            CUSHION.mul_add(2.0, TABLE_SIZE.y),
        ),
        Color::srgb(0.20, 0.075, 0.025),
        Vec3::new(TABLE_CENTER_X, 0.0, 0.0),
    );
    spawn_rectangle(
        &mut commands,
        TABLE_SIZE,
        Color::srgb(0.025, 0.38, 0.21),
        Vec3::new(TABLE_CENTER_X, 0.0, 1.0),
    );

    for pocket in pocket_positions() {
        commands.spawn((
            Sprite::from_color(BLACK, Vec2::splat(42.0)),
            Transform::from_translation(pocket.extend(3.0)),
        ));
    }

    spawn_rack(&mut commands);

    commands.spawn((
        Sprite::from_color(WHITE, Vec2::splat(BALL_RADIUS * 2.0)),
        Transform::from_xyz(-330.0, 0.0, 4.0),
    ));
    commands.spawn((
        Sprite::from_color(Color::srgba(0.95, 0.95, 0.85, 0.68), Vec2::new(370.0, 3.0)),
        Transform::from_xyz(-145.0, 0.0, 6.0),
        AimGuide,
    ));
    commands.spawn((
        Sprite::from_color(Color::srgb(0.72, 0.40, 0.13), Vec2::new(420.0, 11.0)),
        Transform::from_xyz(-552.0, 0.0, 7.0),
        Cue,
    ));

    spawn_rectangle(
        &mut commands,
        Vec2::new(42.0, POWER_BAR_HEIGHT + 14.0),
        Color::srgba(0.02, 0.02, 0.02, 0.72),
        Vec3::new(565.0, 0.0, 5.0),
    );
    commands.spawn((
        Sprite::from_color(
            Color::srgb(0.94, 0.58, 0.10),
            Vec2::new(30.0, POWER_BAR_HEIGHT * DEFAULT_POWER),
        ),
        Transform::from_xyz(565.0, POWER_BAR_HEIGHT * (DEFAULT_POWER - 1.0) / 2.0, 6.0),
        PowerFill,
    ));

    commands.spawn((
        Text::new("PWMTF · browser feasibility table"),
        TextFont::from_font_size(24.0),
        TextColor(Color::srgb(0.92, 0.85, 0.65)),
        Node {
            position_type: PositionType::Absolute,
            top: px(18),
            left: px(24),
            ..default()
        },
    ));
    commands.spawn((
        Text::new("Aim by dragging · drag vertically at the right edge for power"),
        TextFont::from_font_size(17.0),
        TextColor(Color::srgb(0.76, 0.82, 0.78)),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(18),
            left: px(24),
            ..default()
        },
    ));
    commands.spawn((
        Text::new("Rotate your device to landscape"),
        TextFont::from_font_size(36.0),
        TextColor(Color::WHITE),
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(Color::srgba(0.02, 0.06, 0.045, 0.97)),
        GlobalZIndex(100),
        OrientationNotice,
    ));
}

fn spawn_rectangle(commands: &mut Commands, size: Vec2, color: Color, translation: Vec3) {
    commands.spawn((
        Sprite::from_color(color, size),
        Transform::from_translation(translation),
    ));
}

fn pocket_positions() -> [Vec2; 6] {
    let half = TABLE_SIZE / 2.0;
    [
        Vec2::new(TABLE_CENTER_X - half.x, -half.y),
        Vec2::new(TABLE_CENTER_X, -half.y),
        Vec2::new(TABLE_CENTER_X + half.x, -half.y),
        Vec2::new(TABLE_CENTER_X - half.x, half.y),
        Vec2::new(TABLE_CENTER_X, half.y),
        Vec2::new(TABLE_CENTER_X + half.x, half.y),
    ]
}

fn spawn_rack(commands: &mut Commands) {
    const COLORS: [Color; 15] = [
        Color::srgb(0.96, 0.75, 0.08),
        Color::srgb(0.10, 0.30, 0.85),
        Color::srgb(0.88, 0.12, 0.10),
        Color::srgb(0.40, 0.12, 0.58),
        Color::srgb(0.95, 0.38, 0.04),
        Color::srgb(0.08, 0.47, 0.20),
        Color::srgb(0.50, 0.12, 0.08),
        Color::srgb(0.04, 0.04, 0.04),
        Color::srgb(0.96, 0.75, 0.08),
        Color::srgb(0.10, 0.30, 0.85),
        Color::srgb(0.88, 0.12, 0.10),
        Color::srgb(0.40, 0.12, 0.58),
        Color::srgb(0.95, 0.38, 0.04),
        Color::srgb(0.08, 0.47, 0.20),
        Color::srgb(0.50, 0.12, 0.08),
    ];
    let spacing = BALL_RADIUS * 2.05;
    let mut index = 0;
    for column in 0_u8..5 {
        for row in 0_u8..=column {
            let column = f32::from(column);
            let row = f32::from(row);
            let x = (column * spacing).mul_add(0.87, 205.0);
            let y = (row - column / 2.0) * spacing;
            commands.spawn((
                Sprite::from_color(COLORS[index], Vec2::splat(BALL_RADIUS * 2.0)),
                Transform::from_xyz(x, y, 4.0),
            ));
            index += 1;
        }
    }
}

fn input_position(window: &Window, touches: &Touches, input: &mut PrototypeInput) -> Option<Vec2> {
    if let Some(id) = input.active_touch {
        if let Some(touch) = touches.get_pressed(id) {
            return Some(touch.position());
        }
        input.active_touch = None;
    }

    if let Some(touch) = touches.iter().next() {
        input.active_touch = Some(touch.id());
        return Some(touch.position());
    }

    window.cursor_position()
}

fn update_from_pointer(input: &mut PrototypeInput, cursor: Vec2, window_size: Vec2) {
    if window_size.min_element() <= 0.0 {
        return;
    }

    if cursor.x > window_size.x * POWER_ZONE_START {
        input.power = (1.0 - cursor.y / window_size.y).clamp(MIN_POWER, 1.0);
    } else {
        let centered = cursor - window_size / 2.0;
        if centered.length_squared() > 16.0 {
            input.aim_angle = (-centered.y).atan2(centered.x);
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn update_input(
    window: Single<&Window>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    mut input: ResMut<PrototypeInput>,
) {
    if !mouse.pressed(MouseButton::Left) && touches.iter().next().is_none() {
        input.active_touch = None;
        return;
    }

    let Some(cursor) = input_position(&window, &touches, &mut input) else {
        return;
    };
    update_from_pointer(
        &mut input,
        cursor,
        Vec2::new(window.width(), window.height()),
    );
}

#[allow(clippy::needless_pass_by_value)]
fn update_aim(
    input: Res<PrototypeInput>,
    mut cue: Single<&mut Transform, (With<Cue>, Without<AimGuide>)>,
    mut guide: Single<&mut Transform, (With<AimGuide>, Without<Cue>)>,
    mut power: Single<(&mut Sprite, &mut Transform), With<PowerFill>>,
) {
    let cue_ball = Vec2::new(-330.0, 0.0);
    let direction = Vec2::from_angle(input.aim_angle);
    cue.translation = (cue_ball - direction * input.power.mul_add(42.0, 222.0)).extend(7.0);
    cue.rotation = Quat::from_rotation_z(input.aim_angle);
    guide.translation = (cue_ball + direction * 185.0).extend(6.0);
    guide.rotation = Quat::from_rotation_z(input.aim_angle);

    let fill_height = POWER_BAR_HEIGHT * input.power;
    power.0.custom_size = Some(Vec2::new(30.0, fill_height));
    power.1.translation.y = (fill_height - POWER_BAR_HEIGHT) / 2.0;
}

#[allow(clippy::needless_pass_by_value)]
fn update_orientation(
    window: Single<&Window>,
    mut notice: Single<&mut Node, With<OrientationNotice>>,
) {
    notice.display = if window.width() < window.height() {
        Display::Flex
    } else {
        Display::None
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pockets_cover_four_corners_and_two_side_centers() {
        let pockets = pocket_positions();
        assert_eq!(pockets.len(), 6);
        assert!((pockets[0].x - pockets[3].x).abs() < f32::EPSILON);
        assert!((pockets[1].x - pockets[4].x).abs() < f32::EPSILON);
        assert!((pockets[2].x - pockets[5].x).abs() < f32::EPSILON);
        assert!(pockets[..3].iter().all(|pocket| pocket.y < 0.0));
        assert!(pockets[3..].iter().all(|pocket| pocket.y > 0.0));
    }

    #[test]
    fn power_pointer_is_clamped_to_supported_range() {
        let mut input = PrototypeInput::default();
        let size = Vec2::new(1_000.0, 500.0);

        update_from_pointer(&mut input, Vec2::new(900.0, -100.0), size);
        assert!((input.power - 1.0).abs() < f32::EPSILON);

        update_from_pointer(&mut input, Vec2::new(900.0, 1_000.0), size);
        assert!((input.power - MIN_POWER).abs() < f32::EPSILON);
    }

    #[test]
    fn aim_pointer_maps_window_coordinates_to_world_angle() {
        let mut input = PrototypeInput::default();
        let size = Vec2::new(1_000.0, 500.0);

        update_from_pointer(&mut input, Vec2::new(750.0, 250.0), size);
        assert!(input.aim_angle.abs() < f32::EPSILON);

        update_from_pointer(&mut input, Vec2::new(500.0, 125.0), size);
        assert!((input.aim_angle - std::f32::consts::FRAC_PI_2).abs() < f32::EPSILON);
    }
}
