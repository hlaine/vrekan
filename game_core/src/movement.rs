use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::player::Player;
use crate::DeltaSeconds;

#[derive(Component, Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

/// Normalized last-nonzero movement direction. Holds its previous value
/// while stationary, so an idle or attacking entity keeps facing whichever
/// way it last moved — this is what "aimed" means in DESIGN.md's Core loop
/// (facing-based, not independent mouse-look), and applies to both players
/// and enemies (see MECHANICS.md's Combat section).
#[derive(Component, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Facing {
    pub x: f32,
    pub y: f32,
}

impl Default for Facing {
    /// Facing "down" (toward the camera in this top-down game) before
    /// anything has moved yet.
    fn default() -> Self {
        Facing { x: 0.0, y: -1.0 }
    }
}

impl Facing {
    /// Updates to point toward `(x, y)`, normalized, if it's nonzero. A
    /// zero vector means "not moving this tick" and is a no-op.
    pub fn update_from_direction(&mut self, x: f32, y: f32) {
        if x == 0.0 && y == 0.0 {
            return;
        }
        let len = (x * x + y * y).sqrt();
        self.x = x / len;
        self.y = y / len;
    }
}

pub fn movement_system(delta: Res<DeltaSeconds>, mut query: Query<(&mut Position, &Velocity)>) {
    let dt = delta.0;
    for (mut position, velocity) in &mut query {
        position.x += velocity.x * dt;
        position.y += velocity.y * dt;
    }
}

/// Max distance any two party members' positions may diverge. The shared
/// camera's max zoom and the hard leash boundary are the same number (see
/// DESIGN.md's Camera & movement section) — defined once here so the
/// client's zoom calculation and the server's clamp can't drift apart.
pub const LEASH_DISTANCE: f32 = 500.0;

/// Clamps every party member's distance from the party centroid to at most
/// half of `LEASH_DISTANCE`, so by the triangle inequality no two players end
/// up more than `LEASH_DISTANCE` apart. Enforced here, in `game_core`, so it
/// runs as part of the server's authoritative movement resolution rather
/// than as a client-side cosmetic clamp — see DESIGN.md's Camera & movement
/// section for why a client-side-only leash isn't acceptable.
pub fn leash_system(mut players: Query<&mut Position, With<Player>>) {
    let count = players.iter().count();
    if count < 2 {
        return;
    }

    let (sum_x, sum_y) = players.iter().fold((0.0, 0.0), |(sx, sy), position| {
        (sx + position.x, sy + position.y)
    });
    let centroid = Position {
        x: sum_x / count as f32,
        y: sum_y / count as f32,
    };
    let max_radius = LEASH_DISTANCE / 2.0;

    for mut position in &mut players {
        let dx = position.x - centroid.x;
        let dy = position.y - centroid.y;
        let distance = (dx * dx + dy * dy).sqrt();
        if distance > max_radius {
            let scale = max_radius / distance;
            position.x = centroid.x + dx * scale;
            position.y = centroid.y + dy * scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn facing_default_points_down() {
        assert_eq!(Facing::default(), Facing { x: 0.0, y: -1.0 });
    }

    #[test]
    fn facing_updates_and_normalizes_on_nonzero_direction() {
        let mut facing = Facing::default();
        facing.update_from_direction(3.0, 4.0);
        assert!((facing.x - 0.6).abs() < 1e-5);
        assert!((facing.y - 0.8).abs() < 1e-5);
    }

    #[test]
    fn facing_holds_previous_value_on_zero_direction() {
        let mut facing = Facing { x: 1.0, y: 0.0 };
        facing.update_from_direction(0.0, 0.0);
        assert_eq!(facing, Facing { x: 1.0, y: 0.0 });
    }

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

    #[test]
    fn leash_system_is_a_no_op_with_fewer_than_two_players() {
        let mut world = World::new();
        let solo = world
            .spawn((
                Position {
                    x: 10_000.0,
                    y: 0.0,
                },
                Player,
            ))
            .id();

        let _ = world.run_system_once(leash_system);

        assert_eq!(
            *world.get::<Position>(solo).unwrap(),
            Position {
                x: 10_000.0,
                y: 0.0
            }
        );
    }

    #[test]
    fn leash_system_leaves_players_within_range_untouched() {
        let mut world = World::new();
        let a = world.spawn((Position { x: 0.0, y: 0.0 }, Player)).id();
        let b = world.spawn((Position { x: 100.0, y: 0.0 }, Player)).id();

        let _ = world.run_system_once(leash_system);

        assert_eq!(
            *world.get::<Position>(a).unwrap(),
            Position { x: 0.0, y: 0.0 }
        );
        assert_eq!(
            *world.get::<Position>(b).unwrap(),
            Position { x: 100.0, y: 0.0 }
        );
    }

    #[test]
    fn leash_system_pulls_players_back_to_leash_distance_apart() {
        let mut world = World::new();
        let a = world.spawn((Position { x: -400.0, y: 0.0 }, Player)).id();
        let b = world.spawn((Position { x: 400.0, y: 0.0 }, Player)).id();

        let _ = world.run_system_once(leash_system);

        let pos_a = *world.get::<Position>(a).unwrap();
        let pos_b = *world.get::<Position>(b).unwrap();
        assert!((pos_a.distance(&pos_b) - LEASH_DISTANCE).abs() < 1e-4);
        assert!((pos_a.x - (-LEASH_DISTANCE / 2.0)).abs() < 1e-4);
        assert!((pos_b.x - (LEASH_DISTANCE / 2.0)).abs() < 1e-4);
    }
}
