#![cfg_attr(feature = "fail-on-warnings", deny(warnings))]
#![warn(clippy::all, clippy::pedantic, clippy::nursery, clippy::cargo)]
#![allow(clippy::multiple_crate_versions)]

#[cfg(target_arch = "wasm32")]
pub mod browser_transport;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;
pub mod prediction;
pub mod transport;

use bevy::{
    camera::{OrthographicProjection, Projection, ScalingMode},
    color::palettes::css::WHITE,
    prelude::*,
    window::{PresentMode, WindowFocused, WindowResolution},
};

const DESIGN_SIZE: Vec2 = Vec2::new(1280.0, 720.0);
const TABLE_CENTER_X: f32 = -55.0;
const TABLE_SIZE: Vec2 = Vec2::new(960.0, 480.0);
const CUSHION: f32 = 34.0;
const BALL_RADIUS: f32 = 13.0;
const POWER_BAR_HEIGHT: f32 = 300.0;
const POWER_ZONE_START: f32 = 0.82;
const SPIN_ZONE_CENTER: Vec2 = Vec2::new(105.0, 105.0);
const SPIN_ZONE_RADIUS: f32 = 58.0;
const POCKET_SELECTION_RADIUS: f32 = 44.0;
#[cfg(not(target_arch = "wasm32"))]
const _: (f32, f32, f32) = (
    SPIN_ZONE_CENTER.x,
    SPIN_ZONE_RADIUS,
    POCKET_SELECTION_RADIUS,
);
const MIN_POWER: f32 = 0.05;
const DEFAULT_POWER: f32 = 0.55;
const MINIMUM_VIEWPORT_WIDTH: f32 = 1.0;

#[derive(Component)]
struct CanonicalBall(u8);

#[derive(Component)]
struct Cue;

#[derive(Component)]
struct AimGuide;

#[derive(Component)]
struct PowerFill;

#[derive(Component)]
struct OrientationNotice;

#[derive(Component)]
struct TurnStatus;

#[derive(Component)]
struct MatchControlChrome;

#[derive(Resource, Default)]
struct CanonicalPresentation {
    checksum: Option<u64>,
    target: std::collections::BTreeMap<u8, Vec2>,
    pocketed: std::collections::BTreeSet<u8>,
    status: Option<String>,
    active_player: Option<pwmtf_game_domain::Player>,
    completed: bool,
}

#[derive(Resource)]
struct PresentationTier(&'static str);

#[derive(Resource)]
struct PrototypeInput {
    aim_angle: f32,
    power: f32,
    #[cfg(target_arch = "wasm32")]
    spin: Vec2,
    #[cfg(target_arch = "wasm32")]
    called_pocket: Option<pwmtf_game_domain::PocketId>,
    #[cfg(target_arch = "wasm32")]
    placing_cue_ball: bool,
    #[cfg(target_arch = "wasm32")]
    placement_sent_during_contact: bool,
    active_touch: Option<u64>,
    touch_rearm_blocked: bool,
}

impl PrototypeInput {
    const fn release_active_touch(&mut self, another_touch_is_pressed: bool) {
        self.active_touch = None;
        self.touch_rearm_blocked = another_touch_is_pressed;
    }

    const fn rearm_touch_input(&mut self) {
        self.touch_rearm_blocked = false;
    }
}

impl Default for PrototypeInput {
    fn default() -> Self {
        Self {
            aim_angle: 0.25,
            power: DEFAULT_POWER,
            #[cfg(target_arch = "wasm32")]
            spin: Vec2::ZERO,
            #[cfg(target_arch = "wasm32")]
            called_pocket: None,
            #[cfg(target_arch = "wasm32")]
            placing_cue_ball: false,
            #[cfg(target_arch = "wasm32")]
            placement_sent_during_contact: false,
            active_touch: None,
            touch_rearm_blocked: false,
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), allow(clippy::missing_const_for_fn))]
fn presentation_tier() -> &'static str {
    #[cfg(target_arch = "wasm32")]
    {
        let search = web_sys::window()
            .and_then(|window| window.location().search().ok())
            .unwrap_or_default();
        if search
            .trim_start_matches('?')
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .any(|(key, value)| key == "tier" && value == "reduced")
        {
            return "reduced";
        }
    }
    "default"
}

#[cfg(target_arch = "wasm32")]
fn browser_match_requested() -> bool {
    web_sys::window()
        .and_then(|window| window.location().search().ok())
        .is_some_and(|search| {
            search
                .trim_start_matches('?')
                .split('&')
                .filter_map(|pair| pair.split_once('='))
                .any(|(key, value)| key == "match" && !value.is_empty())
        })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Connects the browser to an authorized same-origin match subscription.
///
/// # Errors
///
/// Returns a JavaScript exception when socket construction fails.
pub fn connect_match_socket(url: &str) -> Result<(), wasm_bindgen::JsValue> {
    browser_transport::connect(url)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn disconnect_match_socket() {
    browser_transport::disconnect();
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Returns whether the match socket completed negotiation and initialization.
#[must_use]
pub fn match_socket_ready() -> bool {
    browser_transport::is_ready()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Returns whether the current socket failed and needs a reconnect attempt.
#[must_use]
pub fn match_socket_needs_reconnect() -> bool {
    browser_transport::needs_reconnect()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Returns the retry delay selected by the transport lifecycle.
#[must_use]
pub fn match_socket_retry_delay_ms() -> u64 {
    browser_transport::retry_delay_ms()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Returns the current authoritative/predicted revision for presentation.
#[must_use]
pub fn match_revision() -> Option<u64> {
    browser_transport::authoritative_revision()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Returns whether local presentation awaits authority for one predicted command.
#[must_use]
pub fn match_has_pending_prediction() -> bool {
    browser_transport::has_pending_prediction()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Sets the authenticated participant seat used only to gate local input.
///
/// # Errors
///
/// Returns a JavaScript exception unless `player` is one or two.
pub fn set_match_player(player: u8) -> Result<(), wasm_bindgen::JsValue> {
    let player = match player {
        1 => pwmtf_game_domain::Player::One,
        2 => pwmtf_game_domain::Player::Two,
        _ => return Err(wasm_bindgen::JsValue::from_str("invalid participant seat")),
    };
    browser_transport::set_local_player(player);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Returns whether the local participant may submit active-player commands.
#[must_use]
pub fn match_accepts_active_player_command() -> bool {
    browser_transport::accepts_active_player_command()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Returns and clears whether authority rejected the latest gameplay command.
pub fn match_command_rejected() -> bool {
    browser_transport::take_command_rejected()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Returns whether the authoritative match accepts gameplay commands.
#[must_use]
pub fn match_accepts_gameplay_commands() -> bool {
    browser_transport::accepts_gameplay_commands()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Returns the authoritative active player, or zero after match completion.
#[must_use]
pub fn match_active_player() -> u8 {
    browser_transport::authoritative_match_info().map_or(0, |(player, outcome)| {
        outcome.map_or_else(|| player_number(player), |_| 0)
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Returns the authoritative winner, or zero while the match is active.
#[must_use]
pub fn match_winner() -> u8 {
    browser_transport::authoritative_match_info()
        .and_then(|(_, outcome)| outcome)
        .map_or(0, |outcome| player_number(outcome.winner))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Returns the stable authoritative completion-reason label, or an empty string.
#[must_use]
pub fn match_completion_reason() -> String {
    browser_transport::authoritative_match_info()
        .and_then(|(_, outcome)| outcome)
        .map_or_else(String::new, |outcome| {
            match outcome.reason {
                pwmtf_game_domain::CompletionReason::LegalEightBall => "legal-eight-ball",
                pwmtf_game_domain::CompletionReason::IllegalEightBall => "illegal-eight-ball",
                pwmtf_game_domain::CompletionReason::Concession => "concession",
            }
            .to_owned()
        })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Returns the current canonical predicted checksum for presentation effects.
#[must_use]
pub fn match_checksum() -> Option<u64> {
    browser_transport::predicted_checksum()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Predicts and sends a quantized cue-ball placement.
///
/// # Errors
///
/// Returns a JavaScript exception unless the socket is ready, prediction is
/// valid, secure identifier generation succeeds, and transmission succeeds.
pub fn send_cue_ball_placement(x_micros: i64, y_micros: i64) -> Result<(), wasm_bindgen::JsValue> {
    browser_transport::predict_and_send_cue_ball_placement(
        random_command_id()?,
        pwmtf_game_domain::Vector::from_micros(x_micros, y_micros),
    )
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Predicts and sends a bounded canonical shot.
///
/// # Errors
///
/// Returns a JavaScript exception for invalid bounded input, unavailable socket
/// state, rejected local prediction, identifier generation, or transmission.
pub fn send_shot_command(
    aim_steps: u16,
    power_units: u16,
    spin_side: i16,
    spin_vertical: i16,
    called_pocket: u8,
) -> Result<(), wasm_bindgen::JsValue> {
    let aim = pwmtf_game_domain::Aim::new(aim_steps)
        .map_err(|_| wasm_bindgen::JsValue::from_str("invalid aim"))?;
    let power = pwmtf_game_domain::ShotPower::new(power_units)
        .map_err(|_| wasm_bindgen::JsValue::from_str("invalid power"))?;
    let spin = pwmtf_game_domain::Spin::new(spin_side, spin_vertical)
        .map_err(|_| wasm_bindgen::JsValue::from_str("invalid spin"))?;
    let called_pocket = decode_called_pocket(called_pocket)?;
    browser_transport::predict_and_send_shot(
        random_command_id()?,
        pwmtf_game_domain::VersionedShotCommand::new(aim, power, spin),
        called_pocket,
    )
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
/// Predicts and sends an explicit concession for the authenticated participant seat.
///
/// # Errors
///
/// Returns a JavaScript exception unless the socket is initialized, identifier
/// generation succeeds, prediction is valid, and transmission succeeds.
pub fn send_concession(player: u8) -> Result<(), wasm_bindgen::JsValue> {
    let player = match player {
        1 => pwmtf_game_domain::Player::One,
        2 => pwmtf_game_domain::Player::Two,
        _ => return Err(wasm_bindgen::JsValue::from_str("invalid player seat")),
    };
    browser_transport::predict_and_send_concession(random_command_id()?, player)
}

#[cfg(target_arch = "wasm32")]
fn decode_called_pocket(
    value: u8,
) -> Result<Option<pwmtf_game_domain::PocketId>, wasm_bindgen::JsValue> {
    Ok(match value {
        0 => None,
        1 => Some(pwmtf_game_domain::PocketId::TopLeft),
        2 => Some(pwmtf_game_domain::PocketId::TopCenter),
        3 => Some(pwmtf_game_domain::PocketId::TopRight),
        4 => Some(pwmtf_game_domain::PocketId::BottomLeft),
        5 => Some(pwmtf_game_domain::PocketId::BottomCenter),
        6 => Some(pwmtf_game_domain::PocketId::BottomRight),
        _ => return Err(wasm_bindgen::JsValue::from_str("invalid called pocket")),
    })
}

#[cfg(target_arch = "wasm32")]
fn random_command_id() -> Result<pwmtf_protocol::CommandId, wasm_bindgen::JsValue> {
    let crypto = web_sys::window()
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("window is unavailable"))?
        .crypto()?;
    let mut bytes = [0_u8; 16];
    crypto.get_random_values_with_u8_array(&mut bytes)?;
    Ok(pwmtf_protocol::CommandId::new(bytes))
}

fn main() {
    let presentation_tier = presentation_tier();
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.025, 0.075, 0.055)))
        .insert_resource(PresentationTier(presentation_tier))
        .init_resource::<PrototypeInput>()
        .init_resource::<CanonicalPresentation>()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("Pool with More Than Friends · {presentation_tier} tier"),
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
        .add_systems(
            Update,
            (
                cancel_input_on_focus_loss,
                update_orientation,
                rearm_input_after_valid_landscape,
                update_input,
                update_aim,
                #[cfg(target_arch = "wasm32")]
                synchronize_canonical_presentation,
                update_turn_status,
                update_match_control_visibility,
                interpolate_canonical_balls,
            )
                .chain(),
        )
        .run();
}

#[allow(clippy::needless_pass_by_value)]
fn setup(
    mut commands: Commands,
    presentation_tier: Res<PresentationTier>,
    mut presentation: ResMut<CanonicalPresentation>,
) {
    let initial = pwmtf_game_domain::MatchState::new(
        pwmtf_game_domain::RulesProfile::standard(),
        pwmtf_game_domain::PhysicsProfile::standard(),
        pwmtf_game_domain::TableGeometry::standard(),
        pwmtf_game_domain::RackSeed::new(42),
        pwmtf_game_domain::Player::One,
    )
    .expect("built-in canonical match configuration is valid");
    #[cfg(target_arch = "wasm32")]
    if !browser_match_requested() {
        presentation.project(&initial);
    }
    #[cfg(not(target_arch = "wasm32"))]
    presentation.project(&initial);
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
            CUSHION.mul_add(2.0, TABLE_SIZE.x) + 28.0,
            CUSHION.mul_add(2.0, TABLE_SIZE.y) + 28.0,
        ),
        Color::srgb(0.055, 0.025, 0.012),
        Vec3::new(TABLE_CENTER_X, -8.0, -0.5),
    );
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
    spawn_rectangle(
        &mut commands,
        Vec2::new(TABLE_SIZE.x - 34.0, TABLE_SIZE.y - 34.0),
        Color::srgba(0.10, 0.62, 0.39, 0.16),
        Vec3::new(TABLE_CENTER_X, 0.0, 1.5),
    );

    spawn_table_details(&mut commands);

    for pocket in pocket_positions() {
        commands.spawn((
            Sprite::from_color(Color::srgb(0.015, 0.018, 0.016), Vec2::splat(46.0)),
            Transform::from_translation(pocket.extend(3.0)),
        ));
    }

    spawn_rack(&mut commands);

    commands.spawn((
        Sprite::from_color(WHITE, Vec2::splat(BALL_RADIUS * 2.0)),
        Transform::from_xyz(-330.0, 0.0, 4.0),
        CanonicalBall(0),
    ));
    commands.spawn((
        Sprite::from_color(Color::srgba(0.95, 0.95, 0.85, 0.68), Vec2::new(370.0, 3.0)),
        Transform::from_xyz(-145.0, 0.0, 6.0),
        AimGuide,
        MatchControlChrome,
    ));
    commands.spawn((
        Sprite::from_color(Color::srgb(0.72, 0.40, 0.13), Vec2::new(420.0, 11.0)),
        Transform::from_xyz(-552.0, 0.0, 7.0),
        Cue,
        MatchControlChrome,
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
        MatchControlChrome,
    ));

    spawn_overlay(&mut commands, presentation_tier.0);
}

fn spawn_table_details(commands: &mut Commands) {
    for x in [-360.0_f32, -120.0, 120.0, 360.0] {
        for y in [
            -TABLE_SIZE.y / 2.0 - CUSHION / 2.0,
            TABLE_SIZE.y / 2.0 + CUSHION / 2.0,
        ] {
            commands.spawn((
                Sprite::from_color(Color::srgb(0.93, 0.72, 0.31), Vec2::splat(7.0)),
                Transform::from_xyz(TABLE_CENTER_X + x, y, 2.0)
                    .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_4)),
            ));
        }
    }
}

fn spawn_overlay(commands: &mut Commands, presentation_tier: &str) {
    commands.spawn((
        Text::new(format!(
            "POOL WITH MORE THAN FRIENDS · {presentation_tier} TIER"
        )),
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
        Text::new("Waiting for an authoritative match"),
        TextFont::from_font_size(18.0),
        TextColor(Color::srgb(0.92, 0.85, 0.65)),
        Node {
            position_type: PositionType::Absolute,
            top: px(58),
            left: px(24),
            ..default()
        },
        TurnStatus,
    ));
    commands.spawn((
        Text::new("Drag to aim · pull the right rail for power · release to shoot"),
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

#[allow(clippy::cast_precision_loss)]
fn canonical_to_world(position: pwmtf_game_domain::Vector) -> Vec2 {
    let geometry = pwmtf_game_domain::TableGeometry::standard();
    Vec2::new(
        (position.x.micros() as f32 / geometry.half_width().micros() as f32)
            .mul_add(TABLE_SIZE.x / 2.0, TABLE_CENTER_X),
        position.y.micros() as f32 / geometry.half_height().micros() as f32 * (TABLE_SIZE.y / 2.0),
    )
}

const fn player_number(player: pwmtf_game_domain::Player) -> u8 {
    match player {
        pwmtf_game_domain::Player::One => 1,
        pwmtf_game_domain::Player::Two => 2,
    }
}

fn match_status_text(
    active_player: pwmtf_game_domain::Player,
    outcome: Option<pwmtf_game_domain::MatchOutcome>,
) -> String {
    outcome.map_or_else(
        || format!("Player {} to shoot", player_number(active_player)),
        |outcome| format!("Player {} wins", player_number(outcome.winner)),
    )
}

impl CanonicalPresentation {
    fn project(&mut self, state: &pwmtf_game_domain::MatchState) {
        self.checksum = Some(state.checksum());
        let outcome = match state.status() {
            pwmtf_game_domain::MatchStatus::InProgress => None,
            pwmtf_game_domain::MatchStatus::Completed(outcome) => Some(outcome),
        };
        self.completed = outcome.is_some();
        self.active_player = (!self.completed).then_some(state.active_player());
        self.status = Some(match_status_text(state.active_player(), outcome));
        self.target.clear();
        self.pocketed.clear();
        for ball in state.table().balls() {
            if ball.pocketed {
                self.pocketed.insert(ball.id.number());
            } else {
                self.target
                    .insert(ball.id.number(), canonical_to_world(ball.position));
            }
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
#[cfg(target_arch = "wasm32")]
fn synchronize_canonical_presentation(mut presentation: ResMut<CanonicalPresentation>) {
    let Some(checksum) = browser_transport::predicted_checksum() else {
        return;
    };
    if presentation.checksum == Some(checksum) {
        return;
    }
    presentation.checksum = Some(checksum);
    if let Some((active_player, outcome)) = browser_transport::authoritative_match_info() {
        presentation.completed = outcome.is_some();
        presentation.active_player = (!presentation.completed).then_some(active_player);
        presentation.status = Some(match_status_text(active_player, outcome));
    }
    presentation.target.clear();
    presentation.pocketed.clear();
    for number in 0_u8..=15 {
        let Some((position, pocketed)) = browser_transport::predicted_ball(number) else {
            continue;
        };
        if pocketed {
            presentation.pocketed.insert(number);
        } else {
            presentation
                .target
                .insert(number, canonical_to_world(position));
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn update_turn_status(
    presentation: Res<CanonicalPresentation>,
    mut status: Single<&mut Text, With<TurnStatus>>,
) {
    if presentation.is_changed()
        && let Some(message) = &presentation.status
    {
        status.0.clone_from(message);
    }
}

#[allow(clippy::needless_pass_by_value)]
fn update_match_control_visibility(
    presentation: Res<CanonicalPresentation>,
    mut controls: Query<&mut Visibility, With<MatchControlChrome>>,
) {
    if !presentation.is_changed() {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    let local_may_act = browser_transport::accepts_active_player_command();
    #[cfg(not(target_arch = "wasm32"))]
    let local_may_act = presentation.active_player.is_some();
    let visibility = if local_may_act {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut control in &mut controls {
        *control = visibility;
    }
}

#[allow(clippy::needless_pass_by_value)]
fn interpolate_canonical_balls(
    time: Res<Time>,
    presentation: Res<CanonicalPresentation>,
    mut balls: Query<(&CanonicalBall, &mut Transform, &mut Visibility)>,
) {
    let blend = (time.delta_secs() * 18.0).clamp(0.0, 1.0);
    for (ball, mut transform, mut visibility) in &mut balls {
        if presentation.pocketed.contains(&ball.0) {
            *visibility = Visibility::Hidden;
        } else if let Some(target) = presentation.target.get(&ball.0) {
            *visibility = Visibility::Inherited;
            let current = transform.translation.truncate();
            transform.translation = current.lerp(*target, blend).extend(transform.translation.z);
        }
    }
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
                CanonicalBall(u8::try_from(index + 1).expect("rack has fifteen balls")),
            ));
            index += 1;
        }
    }
}

fn input_position(
    window: &Window,
    mouse_is_pressed: bool,
    touches: &Touches,
    input: &mut PrototypeInput,
) -> Option<Vec2> {
    if let Some(id) = input.active_touch {
        if let Some(touch) = touches.get_pressed(id) {
            return Some(touch.position());
        }
        input.release_active_touch(touches.iter().next().is_some() || mouse_is_pressed);
    }

    if input.touch_rearm_blocked {
        return None;
    }

    if let Some(touch) = touches.iter().next() {
        input.active_touch = Some(touch.id());
        return Some(touch.position());
    }

    mouse_is_pressed.then(|| window.cursor_position()).flatten()
}

#[allow(clippy::needless_pass_by_ref_mut)]
fn update_from_pointer(
    input: &mut PrototypeInput,
    cursor: Vec2,
    window_size: Vec2,
    placement_sent_during_contact: &mut bool,
) {
    #[cfg(not(target_arch = "wasm32"))]
    let _ = placement_sent_during_contact;
    if window_size.min_element() <= 0.0 {
        return;
    }

    if cursor.x > window_size.x * POWER_ZONE_START {
        input.power = (1.0 - cursor.y / window_size.y).clamp(MIN_POWER, 1.0);
    } else {
        #[cfg(target_arch = "wasm32")]
        {
            if input.placing_cue_ball
                && let Some(position) = screen_to_canonical(cursor, window_size)
            {
                if send_cue_ball_placement(position.x.micros(), position.y.micros()).is_ok() {
                    *placement_sent_during_contact = true;
                }
                return;
            }
            let spin_offset = (cursor - SPIN_ZONE_CENTER) / SPIN_ZONE_RADIUS;
            if spin_offset.length_squared() <= 1.0 {
                input.spin = Vec2::new(spin_offset.x, -spin_offset.y);
                return;
            }
            if let Some(pocket) = selected_pocket(cursor, window_size) {
                input.called_pocket = Some(pocket);
                return;
            }
        }
        let centered = cursor - window_size / 2.0;
        if centered.length_squared() > 16.0 {
            input.aim_angle = (-centered.y).atan2(centered.x);
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn screen_to_canonical(cursor: Vec2, window_size: Vec2) -> Option<pwmtf_game_domain::Vector> {
    let scale = (window_size / DESIGN_SIZE).min_element();
    if scale <= 0.0 {
        return None;
    }
    let offset = (window_size - DESIGN_SIZE * scale) / 2.0;
    let design = (cursor - offset) / scale;
    let table_left = TABLE_CENTER_X + DESIGN_SIZE.x / 2.0 - TABLE_SIZE.x / 2.0;
    let table_top = DESIGN_SIZE.y / 2.0 - TABLE_SIZE.y / 2.0;
    if design.x < table_left
        || design.x > table_left + TABLE_SIZE.x
        || design.y < table_top
        || design.y > table_top + TABLE_SIZE.y
    {
        return None;
    }
    let normalized_x = ((design.x - table_left) / TABLE_SIZE.x).mul_add(2.0, -1.0);
    let normalized_y = (1.0 - (design.y - table_top) / TABLE_SIZE.y).mul_add(2.0, -1.0);
    let geometry = pwmtf_game_domain::TableGeometry::standard();
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    let x = (normalized_x * geometry.half_width().micros() as f32).round() as i64;
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    let y = (normalized_y * geometry.half_height().micros() as f32).round() as i64;
    Some(pwmtf_game_domain::Vector::from_micros(x, y))
}

#[cfg(target_arch = "wasm32")]
fn selected_pocket(cursor: Vec2, window_size: Vec2) -> Option<pwmtf_game_domain::PocketId> {
    let scale = (window_size / DESIGN_SIZE).min_element();
    let offset = (window_size - DESIGN_SIZE * scale) / 2.0;
    let design_cursor = (cursor - offset) / scale;
    let screen_pockets = [
        (
            pwmtf_game_domain::PocketId::TopLeft,
            Vec2::new(145.0, 600.0),
        ),
        (
            pwmtf_game_domain::PocketId::TopCenter,
            Vec2::new(625.0, 600.0),
        ),
        (
            pwmtf_game_domain::PocketId::TopRight,
            Vec2::new(1_105.0, 600.0),
        ),
        (
            pwmtf_game_domain::PocketId::BottomLeft,
            Vec2::new(145.0, 120.0),
        ),
        (
            pwmtf_game_domain::PocketId::BottomCenter,
            Vec2::new(625.0, 120.0),
        ),
        (
            pwmtf_game_domain::PocketId::BottomRight,
            Vec2::new(1_105.0, 120.0),
        ),
    ];
    screen_pockets
        .into_iter()
        .find(|(_, position)| design_cursor.distance(*position) <= POCKET_SELECTION_RADIUS)
        .map(|(pocket, _)| pocket)
}

#[allow(clippy::needless_pass_by_value)]
fn cancel_input_on_focus_loss(
    mut focus_events: MessageReader<WindowFocused>,
    mut input: ResMut<PrototypeInput>,
) {
    if focus_events.read().any(|event| !event.focused) {
        input.release_active_touch(true);
    }
}

#[allow(clippy::needless_pass_by_value)]
fn update_input(
    window: Single<&Window>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    mut input: ResMut<PrototypeInput>,
    mut was_pressed: Local<bool>,
) {
    let mouse_is_pressed = mouse.pressed(MouseButton::Left);
    let any_pressed = mouse_is_pressed || touches.iter().next().is_some();
    #[cfg(target_arch = "wasm32")]
    if !browser_transport::accepts_active_player_command() {
        if any_pressed {
            input.release_active_touch(true);
        }
        *was_pressed = any_pressed;
        return;
    }
    if *was_pressed && !any_pressed {
        #[cfg(target_arch = "wasm32")]
        {
            if !input.placement_sent_during_contact {
                let _ = release_shot(&input);
            }
            input.placement_sent_during_contact = false;
        }
    }
    *was_pressed = any_pressed;
    if !any_pressed {
        input.active_touch = None;
        return;
    }

    let Some(cursor) = input_position(&window, mouse_is_pressed, &touches, &mut input) else {
        return;
    };
    let mut placement_sent_during_contact = false;
    update_from_pointer(
        &mut input,
        cursor,
        Vec2::new(window.width(), window.height()),
        &mut placement_sent_during_contact,
    );
    #[cfg(target_arch = "wasm32")]
    {
        input.placement_sent_during_contact |= placement_sent_during_contact;
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = placement_sent_during_contact;
}

#[cfg(target_arch = "wasm32")]
fn release_shot(input: &PrototypeInput) -> Result<(), wasm_bindgen::JsValue> {
    let turns = input.aim_angle.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let aim = (turns * f32::from(pwmtf_game_domain::Aim::STEPS_PER_TURN)).round() as u16
        % pwmtf_game_domain::Aim::STEPS_PER_TURN;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let power =
        (input.power.clamp(0.0, 1.0) * f32::from(pwmtf_game_domain::ShotPower::MAX)).round() as u16;
    #[allow(clippy::cast_possible_truncation)]
    let spin_side = (input.spin.x.clamp(-1.0, 1.0) * 10_000.0).round() as i16;
    #[allow(clippy::cast_possible_truncation)]
    let spin_vertical = (input.spin.y.clamp(-1.0, 1.0) * 10_000.0).round() as i16;
    let called_pocket = input.called_pocket.map_or(0, |pocket| match pocket {
        pwmtf_game_domain::PocketId::TopLeft => 1,
        pwmtf_game_domain::PocketId::TopCenter => 2,
        pwmtf_game_domain::PocketId::TopRight => 3,
        pwmtf_game_domain::PocketId::BottomLeft => 4,
        pwmtf_game_domain::PocketId::BottomCenter => 5,
        pwmtf_game_domain::PocketId::BottomRight => 6,
    });
    send_shot_command(aim, power, spin_side, spin_vertical, called_pocket)
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
    mut input: ResMut<PrototypeInput>,
) {
    let is_portrait = window.width() < window.height();
    notice.display = if is_portrait {
        Display::Flex
    } else {
        Display::None
    };
    #[cfg(target_arch = "wasm32")]
    {
        input.placing_cue_ball = browser_transport::ball_in_hand();
    }
    if is_portrait || window.width() < MINIMUM_VIEWPORT_WIDTH {
        input.release_active_touch(true);
    }
}

fn should_rearm_input(
    window_size: Vec2,
    window_is_focused: bool,
    mouse_is_pressed: bool,
    touch_is_pressed: bool,
) -> bool {
    window_is_focused
        && window_size.x >= window_size.y
        && window_size.x >= MINIMUM_VIEWPORT_WIDTH
        && !mouse_is_pressed
        && !touch_is_pressed
}

#[allow(clippy::needless_pass_by_value)]
fn rearm_input_after_valid_landscape(
    window: Single<&Window>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    mut input: ResMut<PrototypeInput>,
) {
    if should_rearm_input(
        Vec2::new(window.width(), window.height()),
        window.focused,
        mouse.pressed(MouseButton::Left),
        touches.iter().next().is_some(),
    ) {
        input.rearm_touch_input();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_projection_drives_ball_targets_and_pockets() {
        use pwmtf_game_domain::{
            MatchState, PhysicsProfile, Player, RackSeed, RulesProfile, TableGeometry,
        };
        let state = MatchState::new(
            RulesProfile::standard(),
            PhysicsProfile::standard(),
            TableGeometry::standard(),
            RackSeed::new(42),
            Player::One,
        )
        .unwrap();
        let mut presentation = CanonicalPresentation::default();
        presentation.project(&state);
        assert_eq!(presentation.target.len(), 16);
        assert_eq!(presentation.active_player, Some(Player::One));
        assert!(!presentation.completed);
        assert!(presentation.target.contains_key(&0));
        assert!(presentation.pocketed.is_empty());
        assert_eq!(
            canonical_to_world(pwmtf_game_domain::Vector::ZERO),
            Vec2::new(TABLE_CENTER_X, 0.0)
        );
    }

    #[test]
    fn terminal_projection_hides_live_match_chrome() {
        use pwmtf_game_domain::{
            MatchState, PhysicsProfile, Player, RackSeed, RulesProfile, TableGeometry,
        };
        let mut state = MatchState::new(
            RulesProfile::standard(),
            PhysicsProfile::standard(),
            TableGeometry::standard(),
            RackSeed::new(42),
            Player::One,
        )
        .unwrap();
        state.concede(Player::Two).unwrap();
        let mut presentation = CanonicalPresentation::default();
        presentation.project(&state);
        assert!(presentation.completed);
        assert_eq!(presentation.active_player, None);
        assert_eq!(presentation.status.as_deref(), Some("Player 1 wins"));
    }

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
    fn released_touch_requires_all_contacts_to_lift_before_rearming() {
        let mut input = PrototypeInput {
            active_touch: Some(7),
            ..PrototypeInput::default()
        };

        input.release_active_touch(true);
        assert!(input.active_touch.is_none());
        assert!(input.touch_rearm_blocked);

        input.rearm_touch_input();
        assert!(!input.touch_rearm_blocked);
    }

    #[test]
    fn portrait_cancellation_stays_blocked_until_landscape_rearm() {
        assert!(!should_rearm_input(
            Vec2::new(500.0, 900.0),
            true,
            false,
            false
        ));
        assert!(!should_rearm_input(
            Vec2::new(900.0, 500.0),
            false,
            false,
            false
        ));
        assert!(!should_rearm_input(
            Vec2::new(900.0, 500.0),
            true,
            true,
            false
        ));
        assert!(!should_rearm_input(
            Vec2::new(900.0, 500.0),
            true,
            false,
            true
        ));
        assert!(should_rearm_input(
            Vec2::new(900.0, 500.0),
            true,
            false,
            false
        ));
    }

    #[test]
    fn rearm_block_does_not_clear_without_explicit_release() {
        let mut input = PrototypeInput {
            touch_rearm_blocked: true,
            ..PrototypeInput::default()
        };

        assert!(input.touch_rearm_blocked);
        input.rearm_touch_input();
        assert!(!input.touch_rearm_blocked);
    }

    #[test]
    fn mouse_cannot_take_over_until_touch_release_barrier_clears() {
        let mut input = PrototypeInput {
            active_touch: Some(7),
            ..PrototypeInput::default()
        };

        input.release_active_touch(true);
        assert!(input.active_touch.is_none());
        assert!(input.touch_rearm_blocked);
    }

    #[test]
    fn power_pointer_is_clamped_to_supported_range() {
        let mut input = PrototypeInput::default();
        let size = Vec2::new(1_000.0, 500.0);

        let mut placed = false;
        update_from_pointer(&mut input, Vec2::new(900.0, -100.0), size, &mut placed);
        assert!((input.power - 1.0).abs() < f32::EPSILON);

        update_from_pointer(&mut input, Vec2::new(900.0, 1_000.0), size, &mut placed);
        assert!((input.power - MIN_POWER).abs() < f32::EPSILON);
    }

    #[test]
    fn aim_pointer_maps_window_coordinates_to_world_angle() {
        let mut input = PrototypeInput::default();
        let size = Vec2::new(1_000.0, 500.0);

        let mut placed = false;
        update_from_pointer(&mut input, Vec2::new(750.0, 250.0), size, &mut placed);
        assert!(input.aim_angle.abs() < f32::EPSILON);

        update_from_pointer(&mut input, Vec2::new(500.0, 125.0), size, &mut placed);
        assert!((input.aim_angle - std::f32::consts::FRAC_PI_2).abs() < f32::EPSILON);
    }
}
