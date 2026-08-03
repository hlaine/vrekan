pub mod combat;
pub mod enemy;
pub mod movement;
pub mod player;
pub mod progression;
pub mod revive;
pub mod status_effect;

pub use combat::{
    AttackRequested, AttackTimer, CombatStats, DamageType, Health, MeleeAttack, Resistances,
};
pub use enemy::{Aggro, Enemy, EnemyKind};
pub use movement::{leash_system, Facing, MoveSpeed, Position, Velocity, LEASH_DISTANCE};
pub use player::{Downed, Player};
pub use progression::{grant_xp, xp_required, Level, Stats, UnspentStatPoints, XpReward};
pub use revive::{revive_system, Reviving};
pub use status_effect::{
    tick_status_effects, ActiveEffects, EffectDefinition, EffectKind, EffectTarget, StackMode,
    Stat, Stunned,
};

use bevy_ecs::prelude::*;

/// Seconds elapsed since the last tick. Kept as a plain resource (rather than
/// depending on `bevy_time`) so `game_core` stays free of any dependency
/// beyond `bevy_ecs` — the client (and later the server) is responsible for
/// updating this from its own clock each frame/tick.
#[derive(Resource, Default)]
pub struct DeltaSeconds(pub f32);
