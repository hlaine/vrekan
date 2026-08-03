pub mod combat;
pub mod enemy;
pub mod movement;
pub mod player;

pub use combat::{AttackRequested, AttackTimer, Health, MeleeAttack};
pub use enemy::{Aggro, Enemy};
pub use movement::{leash_system, MoveSpeed, Position, Velocity, LEASH_DISTANCE};
pub use player::Player;

use bevy_ecs::prelude::*;

/// Seconds elapsed since the last tick. Kept as a plain resource (rather than
/// depending on `bevy_time`) so `game_core` stays free of any dependency
/// beyond `bevy_ecs` — the client (and later the server) is responsible for
/// updating this from its own clock each frame/tick.
#[derive(Resource, Default)]
pub struct DeltaSeconds(pub f32);
