use bevy_app::{App, Plugin};
use bevy_ecs::prelude::Message;
use bevy_replicon::prelude::*;
use bevy_replicon::shared::backend::connected_client::NetworkId;
use game_core::Position;
use serde::{Deserialize, Serialize};

/// Bump when the wire format changes (replicated component shapes, message
/// shapes) so incompatible client/server builds refuse to connect instead of
/// silently desyncing.
pub const PROTOCOL_ID: u64 = 0;

pub const SERVER_PORT: u16 = 5000;

/// Normalized movement direction sent from client to server every frame.
/// The server scales it by the target entity's own `MoveSpeed` rather than
/// trusting a client-supplied speed.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MoveInput {
    pub x: f32,
    pub y: f32,
}

/// Registers everything that crosses the network, so the client and server
/// binaries can't independently drift on what's replicated or which
/// messages exist between them.
pub struct NetworkPlugin;

impl Plugin for NetworkPlugin {
    fn build(&self, app: &mut App) {
        app.replicate::<Position>()
            .replicate::<NetworkId>()
            .add_client_message::<MoveInput>(Channel::Unreliable);
    }
}
