use bevy_ecs::prelude::*;

use crate::DeltaSeconds;

#[derive(Component, Debug, Default, Clone, Copy, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

impl Position {
    pub fn distance(&self, other: &Position) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

/// Units per second, in each axis. Set directly by input (client) or AI
/// (`game_core`) — `movement_system` only integrates it into `Position`.
#[derive(Component, Debug, Default, Clone, Copy, PartialEq)]
pub struct Velocity {
    pub x: f32,
    pub y: f32,
}

impl Velocity {
    pub const ZERO: Velocity = Velocity { x: 0.0, y: 0.0 };

    /// Returns a velocity pointing from `from` toward `to` at `speed` units
    /// per second, or `ZERO` if the two positions coincide.
    pub fn toward(from: &Position, to: &Position, speed: f32) -> Velocity {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        let len = (dx * dx + dy * dy).sqrt();
        if len == 0.0 {
            return Velocity::ZERO;
        }
        Velocity {
            x: dx / len * speed,
            y: dy / len * speed,
        }
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct MoveSpeed(pub f32);

pub fn movement_system(delta: Res<DeltaSeconds>, mut query: Query<(&mut Position, &Velocity)>) {
    let dt = delta.0;
    for (mut position, velocity) in &mut query {
        position.x += velocity.x * dt;
        position.y += velocity.y * dt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn movement_system_integrates_velocity_over_delta_time() {
        let mut world = World::new();
        world.insert_resource(DeltaSeconds(0.5));
        let entity = world
            .spawn((Position { x: 0.0, y: 0.0 }, Velocity { x: 2.0, y: -4.0 }))
            .id();

        let _ = world.run_system_once(movement_system);

        let position = world.get::<Position>(entity).unwrap();
        assert_eq!(*position, Position { x: 1.0, y: -2.0 });
    }

    #[test]
    fn velocity_toward_points_at_target_scaled_by_speed() {
        let from = Position { x: 0.0, y: 0.0 };
        let to = Position { x: 3.0, y: 4.0 };

        let velocity = Velocity::toward(&from, &to, 10.0);

        assert!((velocity.x - 6.0).abs() < 1e-5);
        assert!((velocity.y - 8.0).abs() < 1e-5);
    }

    #[test]
    fn velocity_toward_coincident_positions_is_zero() {
        let point = Position { x: 5.0, y: 5.0 };
        assert_eq!(Velocity::toward(&point, &point, 10.0), Velocity::ZERO);
    }
}
