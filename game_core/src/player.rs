use bevy_ecs::prelude::*;

/// Marks the single player-controlled entity. Enemy AI targets whichever
/// entity carries this component.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct Player;
