use std::net::{Ipv4Addr, UdpSocket};
use std::time::{Duration, SystemTime};

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::{
    netcode::{NetcodeServerTransport, ServerAuthentication, ServerConfig},
    renet::ConnectionConfig,
    RenetChannelsExt, RenetServer, RepliconRenetPlugins,
};
use game_core::movement::movement_system;
use game_core::{DeltaSeconds, MoveSpeed, Player, Position, Velocity};
use protocol::{MoveInput, NetworkPlugin, PROTOCOL_ID, SERVER_PORT};

const PLAYER_SPEED: f32 = 200.0;
const MAX_CLIENTS: usize = 2;
const TICK_RATE: f64 = 60.0;

fn main() {
    App::new()
        .add_plugins(
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1.0 / TICK_RATE,
            ))),
        )
        .add_plugins((
            bevy::log::LogPlugin::default(),
            StatesPlugin,
            RepliconPlugins,
            RepliconRenetPlugins,
        ))
        .add_plugins(NetworkPlugin)
        .init_resource::<DeltaSeconds>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (update_delta_seconds, apply_move_input, movement_system).chain(),
        )
        .add_observer(on_client_connected)
        .run();
}

fn setup(mut commands: Commands, channels: Res<RepliconChannels>) -> Result<()> {
    let server_channels_config = channels.server_configs();
    let client_channels_config = channels.client_configs();

    let server = RenetServer::new(ConnectionConfig {
        server_channels_config,
        client_channels_config,
        ..Default::default()
    });

    let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, SERVER_PORT))?;
    let server_config = ServerConfig {
        current_time,
        max_clients: MAX_CLIENTS,
        protocol_id: PROTOCOL_ID,
        authentication: ServerAuthentication::Unsecure,
        public_addresses: Default::default(),
    };
    let transport = NetcodeServerTransport::new(server_config, socket)?;

    commands.insert_resource(server);
    commands.insert_resource(transport);

    info!("Vrekan server listening on port {SERVER_PORT}");

    Ok(())
}

/// Every connected client is represented as an entity with `ConnectedClient`;
/// we attach the player's gameplay components directly to that same entity
/// rather than tracking a separate client-to-player mapping.
fn on_client_connected(add: On<Add, ConnectedClient>, mut commands: Commands) {
    commands.entity(add.entity).insert((
        Player,
        Position { x: 0.0, y: 0.0 },
        Velocity::ZERO,
        MoveSpeed(PLAYER_SPEED),
        Replicated,
    ));
}

fn update_delta_seconds(time: Res<Time>, mut delta: ResMut<DeltaSeconds>) {
    delta.0 = time.delta_secs();
}

fn apply_move_input(
    mut inputs: MessageReader<FromClient<MoveInput>>,
    mut players: Query<(&MoveSpeed, &mut Velocity), With<Player>>,
) {
    for input in inputs.read() {
        let Some(entity) = input.client_id.entity() else {
            continue;
        };
        let Ok((speed, mut velocity)) = players.get_mut(entity) else {
            continue;
        };
        velocity.x = input.x * speed.0;
        velocity.y = input.y * speed.0;
    }
}
