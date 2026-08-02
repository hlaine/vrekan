use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::Path;
use std::time::SystemTime;

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon::shared::backend::connected_client::NetworkId;
use bevy_replicon_renet::{
    netcode::{ClientAuthentication, NetcodeClientTransport},
    renet::ConnectionConfig,
    RenetChannelsExt, RenetClient, RepliconRenetPlugins,
};
use content::{load_all_enemy_templates, spawn_enemy};
use game_core::combat::{
    attack_system, death_system, tick_attack_timers, AttackRequested, AttackTimer, Health,
    MeleeAttack,
};
use game_core::enemy::ai_system;
use game_core::movement::{movement_system, Position};
use game_core::player::Player;
use game_core::DeltaSeconds;
use protocol::{MoveInput, NetworkPlugin, PROTOCOL_ID, SERVER_PORT};

const PLAYER_MAX_HEALTH: f32 = 100.0;
const PLAYER_ATTACK_RANGE: f32 = 60.0;
const PLAYER_ATTACK_DAMAGE: f32 = 15.0;
const PLAYER_ATTACK_COOLDOWN: f32 = 0.4;

const ENEMY_TEMPLATES_DIR: &str = "assets/enemies";
// Horizontal spacing between spawned enemies so multiple templates don't
// overlap; kept far enough from the player spawn (origin) that no enemy's
// aggro_range reaches an idle player at startup.
const ENEMY_SPAWN_SPACING: f32 = 150.0;

/// Marks a replicated player entity that isn't ours — rendered, but not a
/// target for our local AI/attack systems (`game_core::Player` is reserved
/// for the one entity we actually control).
#[derive(Component)]
struct RemotePlayer;

/// The client id we authenticated with, so we can tell which replicated
/// player entity (identified by its `NetworkId`) is ours.
#[derive(Resource)]
struct LocalClientId(u64);

/// Our own controlled player entity, once its replicated data has arrived
/// and been identified. `None` until then.
#[derive(Resource, Default)]
struct LocalPlayer(Option<Entity>);

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Vrekan".into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((RepliconPlugins, RepliconRenetPlugins))
        .add_plugins(NetworkPlugin)
        .init_resource::<DeltaSeconds>()
        .init_resource::<LocalPlayer>()
        .add_message::<AttackRequested>()
        .add_systems(Startup, (setup_scene, connect_to_server))
        .add_systems(
            Update,
            (
                update_delta_seconds,
                init_replicated_players,
                player_input_system,
                ai_system,
                movement_system,
                tick_attack_timers,
                attack_system,
                death_system,
                sync_transform_system,
            )
                .chain(),
        )
        .run();
}

fn setup_scene(mut commands: Commands) {
    commands.spawn(Camera2d);

    let templates = load_all_enemy_templates(Path::new(ENEMY_TEMPLATES_DIR))
        .unwrap_or_else(|error| panic!("failed to load enemy templates: {error}"));

    for (index, (_kind, template)) in templates.iter().enumerate() {
        let position = Position {
            x: 200.0 + index as f32 * ENEMY_SPAWN_SPACING,
            y: 100.0,
        };
        let entity = spawn_enemy(&mut commands, template, position);
        commands.entity(entity).insert((
            Sprite::from_color(
                Color::srgb(template.color[0], template.color[1], template.color[2]),
                Vec2::splat(template.size),
            ),
            Transform::from_xyz(position.x, position.y, 0.0),
        ));
    }
}

fn connect_to_server(mut commands: Commands, channels: Res<RepliconChannels>) -> Result<()> {
    let server_channels_config = channels.server_configs();
    let client_channels_config = channels.client_configs();

    let client = RenetClient::new(ConnectionConfig {
        server_channels_config,
        client_channels_config,
        ..Default::default()
    });

    let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
    let client_id = current_time.as_millis() as u64;
    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), SERVER_PORT);
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    let authentication = ClientAuthentication::Unsecure {
        client_id,
        protocol_id: PROTOCOL_ID,
        server_addr,
        user_data: None,
    };
    let transport = NetcodeClientTransport::new(current_time, authentication, socket)?;

    commands.insert_resource(client);
    commands.insert_resource(transport);
    commands.insert_resource(LocalClientId(client_id));

    info!("connecting to {server_addr}");

    Ok(())
}

/// Reacts to newly-replicated player entities: the one matching our own
/// `LocalClientId` becomes our controlled `Player` (with local combat
/// components, since combat isn't networked until M4); everyone else is
/// just a `RemotePlayer` we render but don't otherwise act on.
fn init_replicated_players(
    mut commands: Commands,
    my_client_id: Res<LocalClientId>,
    mut local_player: ResMut<LocalPlayer>,
    new_players: Query<(Entity, &NetworkId), Added<NetworkId>>,
) {
    for (entity, network_id) in &new_players {
        if network_id.get() == my_client_id.0 {
            local_player.0 = Some(entity);
            commands.entity(entity).insert((
                Player,
                Health::new(PLAYER_MAX_HEALTH),
                MeleeAttack {
                    range: PLAYER_ATTACK_RANGE,
                    damage: PLAYER_ATTACK_DAMAGE,
                    cooldown: PLAYER_ATTACK_COOLDOWN,
                },
                AttackTimer(0.0),
                Sprite::from_color(Color::srgb(0.2, 0.7, 0.3), Vec2::splat(32.0)),
                Transform::default(),
            ));
        } else {
            commands.entity(entity).insert((
                RemotePlayer,
                Sprite::from_color(Color::srgb(0.3, 0.5, 0.8), Vec2::splat(32.0)),
                Transform::default(),
            ));
        }
    }
}

fn update_delta_seconds(time: Res<Time>, mut delta: ResMut<DeltaSeconds>) {
    delta.0 = time.delta_secs();
}

fn player_input_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    local_player: Res<LocalPlayer>,
    mut move_input: MessageWriter<MoveInput>,
    mut attack_events: MessageWriter<AttackRequested>,
) {
    let Some(entity) = local_player.0 else {
        return;
    };

    let mut direction = Vec2::ZERO;
    if keyboard.pressed(KeyCode::KeyW) {
        direction.y += 1.0;
    }
    if keyboard.pressed(KeyCode::KeyS) {
        direction.y -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyA) {
        direction.x -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyD) {
        direction.x += 1.0;
    }
    let direction = direction.normalize_or_zero();

    move_input.write(MoveInput {
        x: direction.x,
        y: direction.y,
    });

    if keyboard.just_pressed(KeyCode::Space) {
        attack_events.write(AttackRequested { attacker: entity });
    }
}

fn sync_transform_system(mut query: Query<(&Position, &mut Transform)>) {
    for (position, mut transform) in &mut query {
        transform.translation.x = position.x;
        transform.translation.y = position.y;
    }
}
