//! Disposable practice state; never participates in multiplayer transport.
use bevy::prelude::Resource;
use pwmtf_game_domain::{
    BallId, BallState, PhysicsProfile, RackSeed, TableGeometry, Vector, VersionedShotCommand,
    VersionedTableState, advance_tick, standard_rack, start_shot,
};

#[derive(Resource)]
pub struct Sandbox {
    pub enabled: bool,
    previous: VersionedTableState,
    pub table: VersionedTableState,
    pub moving: bool,
    elapsed: f64,
    ticks: u32,
    seed: u64,
}

impl Default for Sandbox {
    fn default() -> Self {
        Self {
            enabled: practice_enabled(),
            previous: rack(42),
            table: rack(42),
            moving: false,
            elapsed: 0.0,
            ticks: 0,
            seed: 42,
        }
    }
}

// Browser URL inspection cannot be const, even though the native fallback is.
#[allow(clippy::missing_const_for_fn)]
fn practice_enabled() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|window| window.location().search().ok())
            .is_some_and(|search| {
                !search
                    .trim_start_matches('?')
                    .split('&')
                    .any(|part| part.split('=').next() == Some("match"))
            })
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        true
    }
}

fn rack(seed: u64) -> VersionedTableState {
    standard_rack(TableGeometry::standard(), RackSeed::new(seed))
        .expect("standard practice rack is valid")
}

impl Sandbox {
    pub fn shoot(&mut self, command: VersionedShotCommand) {
        if !self.enabled || self.moving {
            return;
        }
        if let Ok(table) = start_shot(PhysicsProfile::standard(), self.table.clone(), command) {
            self.previous = table.clone();
            self.table = table;
            self.moving = true;
            self.elapsed = 0.0;
            self.ticks = 0;
        }
    }

    pub fn update(&mut self, seconds: f64) {
        if !self.enabled || !self.moving {
            return;
        }
        let profile = PhysicsProfile::standard();
        let step = 1.0 / f64::from(profile.ticks_per_second());
        self.elapsed += seconds.min(0.1);
        while self.elapsed >= step && self.moving {
            self.elapsed -= step;
            self.ticks += 1;
            self.previous = self.table.clone();
            match advance_tick(TableGeometry::standard(), profile, self.table.clone()) {
                Ok(result) => {
                    self.table = result.state;
                    if result.settled {
                        self.moving = false;
                        self.finish_rack_or_restore_cue();
                    }
                }
                Err(_) => self.reset(),
            }
            if self.ticks >= profile.maximum_ticks() {
                self.reset();
            }
        }
    }

    /// Interpolates between adjacent physics ticks rather than chasing state.
    pub fn presentation_position(
        &self,
        id: pwmtf_game_domain::BallId,
    ) -> Option<bevy::prelude::Vec2> {
        let current = self.table.ball(id)?;
        let target = crate::canonical_to_world(current.position);
        if !self.moving {
            return Some(target);
        }
        let previous = self.previous.ball(id)?;
        #[allow(clippy::cast_possible_truncation)]
        let alpha = (self.elapsed * f64::from(PhysicsProfile::standard().ticks_per_second()))
            .clamp(0.0, 1.0) as f32;
        Some(crate::canonical_to_world(previous.position).lerp(target, alpha))
    }

    fn reset(&mut self) {
        self.seed = self.seed.wrapping_add(1);
        self.table = rack(self.seed);
        self.previous = self.table.clone();
        self.moving = false;
        self.ticks = 0;
    }

    fn finish_rack_or_restore_cue(&mut self) {
        if self
            .table
            .balls()
            .iter()
            .all(|ball| ball.id == BallId::CUE || ball.pocketed)
        {
            self.reset();
            return;
        }
        if !self
            .table
            .ball(BallId::CUE)
            .is_some_and(|ball| ball.pocketed)
        {
            return;
        }
        let geometry = TableGeometry::standard();
        let diameter = geometry.ball_radius().micros() * 2;
        let mut balls = self.table.balls().to_vec();
        // Search the playable area deterministically, rejecting overlap through
        // the domain constructor rather than duplicating collision rules.
        for x in -20..=20 {
            for y in -9..=9 {
                balls[0] = BallState::stationary(
                    BallId::CUE,
                    Vector::from_micros(x * diameter, y * diameter),
                );
                if let Ok(table) =
                    VersionedTableState::new(self.table.version(), geometry, 0, balls.clone())
                {
                    self.table = table;
                    return;
                }
            }
        }
        self.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pwmtf_game_domain::{Aim, ShotPower, Spin};

    #[test]
    fn repeated_shots_settle_without_turns() {
        let mut sandbox = Sandbox::default();
        for _ in 0..3 {
            sandbox.shoot(VersionedShotCommand::new(
                Aim::new(0).unwrap(),
                ShotPower::new(5_000).unwrap(),
                Spin::CENTER,
            ));
            assert!(sandbox.moving);
            for _ in 0..10_000 {
                sandbox.update(0.1);
                if !sandbox.moving {
                    break;
                }
            }
            assert!(!sandbox.moving);
            assert!(!sandbox.table.ball(BallId::CUE).unwrap().pocketed);
        }
    }

    #[test]
    fn scratch_restores_cue_and_clear_table_reracks() {
        let mut sandbox = Sandbox::default();
        let mut balls = sandbox.table.balls().to_vec();
        balls[0].pocketed = true;
        sandbox.table = VersionedTableState::new(
            sandbox.table.version(),
            TableGeometry::standard(),
            0,
            balls.clone(),
        )
        .unwrap();
        sandbox.finish_rack_or_restore_cue();
        assert!(!sandbox.table.ball(BallId::CUE).unwrap().pocketed);
        for ball in &mut balls {
            ball.pocketed = true;
        }
        sandbox.table =
            VersionedTableState::new(sandbox.table.version(), TableGeometry::standard(), 0, balls)
                .unwrap();
        sandbox.finish_rack_or_restore_cue();
        assert!(sandbox.table.balls().iter().all(|ball| !ball.pocketed));
        assert_eq!(sandbox.seed, 43);
    }
}
