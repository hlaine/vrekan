use bevy_app::{App, Plugin};
use bevy_ecs::prelude::Message;
use bevy_replicon::prelude::*;
use bevy_replicon::shared::backend::connected_client::NetworkId;
use game_core::{Downed, Enemy, EnemyKind, Facing, Health, Level, Position, Stats, Stunned};
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

/// Fixed size of netcode's connection-time `user_data` field (see
/// `renetcode::NETCODE_USER_DATA_BYTES`, verified as 256 in that crate's
/// source). `protocol` doesn't depend on `renet` directly, so this is a
/// local mirror of that number rather than a new dependency for one
/// constant — if `renet`/`renetcode` ever changes it, `ConnectAuth::encode`
/// would start failing loudly (`TooLarge`) well before anything silently
/// corrupts, since encoding checks the length explicitly.
pub const CONNECT_AUTH_BYTES: usize = 256;

/// Carried in netcode's connection-time `user_data` field, not a gameplay
/// message — this is authentication metadata the server needs *before* it
/// finishes setting up a connecting client's entity, not something that
/// flows through the normal replicated-message pipeline. `character_id` is
/// a client-generated, locally-persisted identifier (not tied to any
/// account system); a character is scoped to one game (one server
/// process), not portable across different games — see DECISIONS.md.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectAuth {
    pub game_password: String,
    pub character_id: u128,
    pub character_password: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectAuthError {
    /// The encoded RON text didn't fit in `CONNECT_AUTH_BYTES`.
    TooLarge,
    /// Not valid UTF-8, or not parseable as `ConnectAuth` RON.
    Malformed,
}

impl std::fmt::Display for ConnectAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectAuthError::TooLarge => {
                write!(f, "encoded ConnectAuth exceeds {CONNECT_AUTH_BYTES} bytes")
            }
            ConnectAuthError::Malformed => write!(f, "ConnectAuth bytes are not valid RON"),
        }
    }
}

impl std::error::Error for ConnectAuthError {}

impl ConnectAuth {
    /// RON rather than a hand-rolled byte layout — this is a small, rarely
    /// (dis)assembled struct, not a hot-path wire message, so reusing
    /// `serde`/`ron` (already workspace dependencies) is simpler than
    /// maintaining manual field offsets.
    pub fn encode(&self) -> Result<[u8; CONNECT_AUTH_BYTES], ConnectAuthError> {
        let text = ron::to_string(self).map_err(|_| ConnectAuthError::Malformed)?;
        let bytes = text.as_bytes();
        if bytes.len() > CONNECT_AUTH_BYTES {
            return Err(ConnectAuthError::TooLarge);
        }
        let mut buffer = [0u8; CONNECT_AUTH_BYTES];
        buffer[..bytes.len()].copy_from_slice(bytes);
        Ok(buffer)
    }

    /// Trims trailing zero-padding before parsing — `encode` zero-pads, and
    /// `0` can't appear inside the RON text itself (not valid UTF-8
    /// mid-string for our field types), so the first zero byte reliably
    /// marks the end of the real payload.
    pub fn decode(bytes: &[u8; CONNECT_AUTH_BYTES]) -> Result<Self, ConnectAuthError> {
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        let text = std::str::from_utf8(&bytes[..end]).map_err(|_| ConnectAuthError::Malformed)?;
        ron::from_str(text).map_err(|_| ConnectAuthError::Malformed)
    }
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
            .replicate::<Level>()
            .replicate::<Stats>()
            .add_client_message::<MoveInput>(Channel::Unreliable)
            .add_client_message::<AttackInput>(Channel::Unreliable)
            .add_client_message::<ReviveInput>(Channel::Unreliable);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth() -> ConnectAuth {
        ConnectAuth {
            game_password: "hunter2".to_string(),
            character_id: 123456789,
            character_password: "correct-horse".to_string(),
        }
    }

    #[test]
    fn connect_auth_round_trips_through_encode_and_decode() {
        let encoded = auth().encode().unwrap();
        let decoded = ConnectAuth::decode(&encoded).unwrap();
        assert_eq!(decoded, auth());
    }

    #[test]
    fn connect_auth_encode_rejects_a_payload_too_large_to_fit() {
        let oversized = ConnectAuth {
            game_password: "x".repeat(CONNECT_AUTH_BYTES),
            character_id: 0,
            character_password: String::new(),
        };
        assert_eq!(oversized.encode().unwrap_err(), ConnectAuthError::TooLarge);
    }

    #[test]
    fn connect_auth_decode_rejects_garbage_bytes() {
        let garbage = [0xFFu8; CONNECT_AUTH_BYTES];
        assert_eq!(
            ConnectAuth::decode(&garbage).unwrap_err(),
            ConnectAuthError::Malformed
        );
    }
}
