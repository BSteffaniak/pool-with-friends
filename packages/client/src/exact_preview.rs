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
}

impl ExactPreview {
    pub fn draw(
        &mut self,
        gizmos: &mut Gizmos,
        table: VersionedTableState,
        physics: PhysicsProfile,
        geometry: TableGeometry,
        shot: VersionedShotCommand,
    ) {
        let key = (table, physics, geometry, shot);
        if self.key.as_ref() != Some(&key) {
            self.moving = start_shot(physics, key.0.clone(), shot).ok();
            self.key = Some(key);
            self.ticks = 0;
            self.lines.clear();
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
            for (a, b) in before.balls().iter().zip(result.state.balls()) {
                if !a.pocketed && !b.pocketed && a.position != b.position {
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
        for &(number, a, b) in &self.lines {
            let color = if number == 0 {
                Color::srgba(0.96, 0.96, 0.86, 0.75)
            } else {
                Color::srgb(0.95, 0.72, 0.25)
            };
            gizmos.line_2d(a, b, color);
        }
    }
}
