//! Bounded, incremental previews using the canonical solver, never authority.
use bevy::prelude::*;
use pwmtf_game_domain::{
    PhysicsProfile, TableGeometry, VersionedShotCommand, VersionedTableState, advance_tick,
    start_shot,
};

#[derive(Resource, Default)]
pub struct ExactPreview {
    key: Option<(
        VersionedTableState,
        PhysicsProfile,
        TableGeometry,
        VersionedShotCommand,
    )>,
    moving: Option<VersionedTableState>,
    ticks: u32,
    lines: Vec<(u8, Vec2, Vec2)>,
    short_lines: Vec<(u8, Vec2, Vec2)>,
    contact: Option<Vec2>,
    object: Option<u8>,
    lengths: [f32; 16],
}

fn clip_segment(start: Vec2, end: Vec2, used: &mut f32, budget: f32) -> Option<Vec2> {
    let length = start.distance(end);
    let remaining = (budget - *used).max(0.0);
    if length <= f32::EPSILON || remaining <= 0.0 {
        return None;
    }
    let take = length.min(remaining);
    *used += take;
    Some(start.lerp(end, take / length))
}

impl ExactPreview {
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        gizmos: &mut Gizmos,
        table: VersionedTableState,
        physics: PhysicsProfile,
        geometry: TableGeometry,
        shot: VersionedShotCommand,
        compact: bool,
    ) {
        let key = (table, physics, geometry, shot);
        if self.key.as_ref() != Some(&key) {
            self.moving = start_shot(physics, key.0.clone(), shot).ok();
            self.key = Some(key);
            self.ticks = 0;
            self.lines.clear();
            self.short_lines.clear();
            self.contact = None;
            self.object = None;
            self.lengths = [0.0; 16];
        }
        // A short trajectory, generated in bounded work slices while aiming.
        // Only complete ticks are drawn; never substitute a geometric estimate.
        for _ in 0..24 {
            let Some(before) = self.moving.take() else {
                break;
            };
            let Ok(result) = advance_tick(geometry, physics, before.clone()) else {
                break;
            };
            self.ticks += 1;
            if self.contact.is_none() {
                for event in &result.events {
                    if let pwmtf_game_domain::SimulationEventKind::BallContact { first, second } =
                        event.kind
                        && (first.number() == 0 || second.number() == 0)
                    {
                        self.object = Some(if first.number() == 0 {
                            second.number()
                        } else {
                            first.number()
                        });
                        self.contact = result
                            .state
                            .balls()
                            .iter()
                            .find(|ball| ball.id.number() == 0)
                            .map(|ball| crate::canonical_to_world(ball.position));
                        break;
                    }
                    if matches!(event.kind, pwmtf_game_domain::SimulationEventKind::CushionContact { ball, .. } if ball.number() == 0)
                    {
                        self.contact = result
                            .state
                            .balls()
                            .iter()
                            .find(|ball| ball.id.number() == 0)
                            .map(|ball| crate::canonical_to_world(ball.position));
                        break;
                    }
                }
            }
            for (a, b) in before.balls().iter().zip(result.state.balls()) {
                if !a.pocketed && !b.pocketed && a.position != b.position {
                    if self.contact.is_some()
                        && (a.id.number() == 0 || Some(a.id.number()) == self.object)
                    {
                        let number = a.id.number();
                        let start = crate::canonical_to_world(a.position);
                        let end = crate::canonical_to_world(b.position);
                        let budget = if number == 0 { 80.0 } else { 100.0 };
                        if let Some(end) =
                            clip_segment(start, end, &mut self.lengths[usize::from(number)], budget)
                        {
                            self.short_lines.push((number, start, end));
                        }
                    }
                    self.lines.push((
                        a.id.number(),
                        crate::canonical_to_world(a.position),
                        crate::canonical_to_world(b.position),
                    ));
                }
            }
            if !result.settled && self.ticks < u32::from(physics.ticks_per_second()) * 3 {
                self.moving = Some(result.state);
            }
        }
        if compact {
            self.draw_incoming(gizmos);
        }
        let lines = if compact {
            &self.short_lines
        } else {
            &self.lines
        };
        for &(number, a, b) in lines {
            let color = if number == 0 {
                Color::srgba(0.96, 0.96, 0.86, 0.75)
            } else {
                Color::srgb(0.95, 0.72, 0.25)
            };
            gizmos.line_2d(a, b, color);
        }
    }
    fn draw_incoming(&self, gizmos: &mut Gizmos) {
        if let Some((table, _, _, _)) = &self.key
            && let Some(cue) = table.balls().iter().find(|ball| ball.id.number() == 0)
        {
            let origin = crate::canonical_to_world(cue.position);
            let endpoint = self.contact.or_else(|| {
                self.lines
                    .iter()
                    .rev()
                    .find(|line| line.0 == 0)
                    .map(|line| line.2)
            });
            if let Some(endpoint) = endpoint {
                let direction = (endpoint - origin).normalize_or_zero();
                let length = origin.distance(endpoint).min(500.0);
                if length > crate::BALL_RADIUS {
                    gizmos.line_2d(
                        origin + direction * crate::BALL_RADIUS,
                        origin + direction * length,
                        Color::srgba(0.96, 0.96, 0.86, 0.75),
                    );
                }
                if self.object.is_some() {
                    gizmos.circle_2d(endpoint, crate::BALL_RADIUS, Color::WHITE);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn short_paths_clip_at_the_display_budget() {
        let mut used = 0.0;
        assert_eq!(
            clip_segment(Vec2::ZERO, Vec2::new(60.0, 0.0), &mut used, 80.0),
            Some(Vec2::new(60.0, 0.0))
        );
        assert_eq!(
            clip_segment(Vec2::new(60.0, 0.0), Vec2::new(120.0, 0.0), &mut used, 80.0),
            Some(Vec2::new(80.0, 0.0))
        );
        assert!(clip_segment(Vec2::ZERO, Vec2::X, &mut used, 80.0).is_none());
    }
}
