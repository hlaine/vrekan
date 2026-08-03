use bevy_app::{App, Plugin};
use bevy_ecs::prelude::Message;
use bevy_replicon::prelude::*;
use bevy_replicon::shared::backend::connected_client::NetworkId;
use game_core::{Downed, Enemy, EnemyKind, Facing, Health, Position, Stunned};
use serde::{Deserialize, Serialize};

/// Bump when the wire format changes (replicated component shapes, message
/// shapes) so incompatible client/server builds refuse to connect instead of
/// silently desyncing.
pub const PROTOCOL_ID: u64 = 1;

pub const SERVER_PORT: u16 = 5000;

/// Normalized movement direction sent from client to server every frame.
/// The server scales it by the target entity's own `MoveSpeed` rather than
/// trusting a client-supplied speed.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MoveInput {
    pub x: f32,
    pub y: f32,
}

/// Sent when the player presses the melee-attack button. No payload — the
/// server resolves the actual attack (range, cooldown, target, damage)
/// against its own authoritative state, never trusting client-supplied
/// combat outcomes.
///
/// Uses `Channel::Unreliable`, not a reliable channel, despite being a
/// discrete one-shot action rather than continuous state — see
/// `DECISIONS.md` for why: `Channel::Unordered` was tried first and
/// reproducibly stopped delivering messages after the first ~8 in live
/// testing (confirmed via client/server logs — the client kept sending,
/// the server just stopped receiving), while `Unreliable` delivered every
/// message across repeated tests. Treat this as an occasionally-dropped
/// input, same as `MoveInput`, not a guaranteed delivery.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttackInput;

/// Sent every frame while the player holds the revive/interact button, same
/// continuous-state pattern as `MoveInput` — a dropped frame self-corrects
/// on the next one, so `Channel::Unreliable` is fine here too (see
/// `AttackInput`'s doc comment for why the "reliable" channels in this
/// dependency stack aren't actually worth reaching for even for a discrete
/// action, let alone a continuous one like this).
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReviveInput {
    pub held: bool,
}

/// Registers everything that crosses the network, so the client and server
/// binaries can't independently drift on what's replicated or which
/// messages exist between them.
pub struct NetworkPlugin;

impl Plugin for NetworkPlugin {
    fn build(&self, app: &mut App) {
        app.replicate::<Position>()
            .replicate::<NetworkId>()
            .replicate::<Health>()
            .replicate::<Enemy>()
            .replicate::<EnemyKind>()
            .replicate::<Downed>()
            .replicate::<Facing>()
            .replicate::<Stunned>()
            .add_client_message::<MoveInput>(Channel::Unreliable)
            .add_client_message::<AttackInput>(Channel::Unreliable)
            .add_client_message::<ReviveInput>(Channel::Unreliable);
    }
}
