use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

/// Marks the single player-controlled entity. Enemy AI targets whichever
/// entity carries this component.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct Player;

/// Marks a player entity that has taken lethal damage but hasn't been
/// despawned — incapacitated and out of combat, waiting for a teammate to
/// revive them (see MECHANICS.md's Death, downed state, and revive
/// section). Enemies have no equivalent: `death_system` despawns them
/// outright on zero health, same as always. Replicated so a client can
/// render its own downed state distinctly rather than the symptom reading
/// as "input stopped working" (see DECISIONS.md).
#[derive(Component, Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct Downed;
