use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

/// A crate/barrel-style world object: has `Health` and can be broken for
/// loot, but no AI, no attack, no `XpReward` — see MECHANICS.md's Dynamic
/// objects section. Deliberately a distinct marker from `Enemy` rather than
/// reusing it: destructibles are spawned from their own
/// `content::DestructibleTemplate`/`spawn_destructible`, not
/// `spawn_enemy`, so nothing that queries `With<Enemy>` (aggro/AI, the M8.8
/// crit-flash/bleeding visuals) accidentally picks them up.
#[derive(Component, Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct Destructible;

/// Which content template (`content::DestructibleTemplate`, keyed by
/// filename stem) this destructible was spawned from — same replicated
/// "client looks up its own already-loaded copy" shape as `EnemyKind`.
#[derive(Component, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DestructibleKind(pub String);
