use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::Path;
use std::time::SystemTime;

use bevy::camera::Projection;
use bevy::prelude::*;
use bevy_ecs_tiled::prelude::{TiledMap, TiledPlugin, TilemapAnchor};
use bevy_replicon::prelude::*;
use bevy_replicon::shared::backend::connected_client::NetworkId;
use bevy_replicon_renet::{
    netcode::{ClientAuthentication, NetcodeClientTransport},
    renet::ConnectionConfig,
    RenetChannelsExt, RenetClient, RepliconRenetPlugins,
};
use content::{load_all_enemy_templates, EnemyTemplate};
use game_core::movement::Position;
use game_core::player::Player;
use game_core::{DeltaSeconds, Downed, Enemy, EnemyKind, LEASH_DISTANCE};
use protocol::{AttackInput, MoveInput, NetworkPlugin, ReviveInput, PROTOCOL_ID, SERVER_PORT};

const PLAYER_COLOR: Color = Color::srgb(0.2, 0.7, 0.3);
const REMOTE_PLAYER_COLOR: Color = Color::srgb(0.3, 0.5, 0.8);

// Only used to look up appearance (color/size) for a replicated enemy by
// its `EnemyKind` — the server is what actually spawns/simulates enemies
// now (see `server/src/main.rs`'s `spawn_enemies`).
const ENEMY_TEMPLATES_DIR: &str = "assets/enemies";

// Relative to the assets root (loaded via AssetServer), not the filesystem
// path used for ENEMY_TEMPLATES_DIR above. Must stay in sync with the
// server's MAP_PATH — see server/src/main.rs's spawn_map_colliders doc
// comment for the anchor/coordinate convention this depends on.
const MAP_PATH: &str = "maps/valley.tmx";

// Orthographic projection scale at zero and max party spread, respectively —
// tuning constants, not derived from anything.
const MIN_ZOOM: f32 = 0.7;
const MAX_ZOOM: f32 = 1.6;

// Fraction of the leash radius (LEASH_DISTANCE / 2) at which the local
// player's leash-limit indicator kicks in.
const LEASH_WARNING_RATIO: f32 = 0.9;
const PLAYER_LEASH_WARNING_COLOR: Color = Color::srgb(0.9, 0.15, 0.15);

// Downed is a distinct, unmissable grey — deliberately not just a darker
// shade of the leash-warning red, since a downed player and a
// near-the-leash player need to read as different situations at a glance.
const DOWNED_COLOR: Color = Color::srgb(0.5, 0.5, 0.5);

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

/// Enemy templates loaded purely for appearance lookup (color/size) by
/// `EnemyKind` — see `init_replicated_enemies`. Simulation-relevant fields
/// (health, damage, AI ranges) only matter server-side now.
#[derive(Resource)]
struct EnemyTemplates(Vec<(String, EnemyTemplate)>);

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Vrekan".into(),
                        ..default()
                    }),
                    ..default()
                })
                // Bevy's default asset root resolves relative to this crate's
                // own manifest directory (a `cargo run`-from-anywhere
                // convenience), but this project keeps one shared `assets/`
                // folder at the workspace root (already used by `content`'s
                // direct filesystem loading) — point the asset server there.
                .set(AssetPlugin {
                    file_path: "../assets".into(),
                    ..default()
                }),
        )
        .add_plugins((RepliconPlugins, RepliconRenetPlugins))
        .add_plugins(NetworkPlugin)
        .add_plugins(TiledPlugin::default())
        .init_resource::<DeltaSeconds>()
        .init_resource::<LocalPlayer>()
        .add_systems(Startup, (setup_scene, connect_to_server))
        .add_systems(
            Update,
            (
                update_delta_seconds,
                init_replicated_players,
                init_replicated_enemies,
                player_input_system,
                sync_transform_system,
                party_camera_system,
                player_appearance_system,
            )
                .chain(),
        )
        .run();
}

fn setup_scene(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn(Camera2d);

    commands.spawn((
        TiledMap(asset_server.load(MAP_PATH)),
        TilemapAnchor::TopLeft,
    ));

    let templates = load_all_enemy_templates(Path::new(ENEMY_TEMPLATES_DIR))
        .unwrap_or_else(|error| panic!("failed to load enemy templates: {error}"));
    commands.insert_resource(EnemyTemplates(templates));
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
/// `LocalClientId` becomes our controlled `Player`; everyone else is just a
/// `RemotePlayer` we render but don't otherwise act on. Combat components
/// (`Health`, `MeleeAttack`, `AttackTimer`) live server-side only now —
/// `Health` replicates in on its own, the rest never need to exist
/// client-side (see `server/src/main.rs`'s `on_client_connected`).
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
                Sprite::from_color(PLAYER_COLOR, Vec2::splat(32.0)),
                Transform::default(),
            ));
        } else {
            commands.entity(entity).insert((
                RemotePlayer,
                Sprite::from_color(REMOTE_PLAYER_COLOR, Vec2::splat(32.0)),
                Transform::default(),
            ));
        }
    }
}

/// Query filter matching newly-replicated enemy entities.
type NewEnemies<'w, 's> =
    Query<'w, 's, (Entity, &'static EnemyKind), (With<Enemy>, Added<EnemyKind>)>;

/// Reacts to newly-replicated enemy entities (spawned server-side — see
/// `server/src/main.rs`'s `spawn_enemies`), giving each one the appearance
/// its `EnemyKind` maps to in our locally-loaded `EnemyTemplates`. Enemy
/// AI/combat is fully server-authoritative; the client only renders them.
fn init_replicated_enemies(
    mut commands: Commands,
    templates: Res<EnemyTemplates>,
    new_enemies: NewEnemies,
) {
    for (entity, kind) in &new_enemies {
        let Some((_, template)) = templates.0.iter().find(|(k, _)| *k == kind.0) else {
            continue;
        };
        commands.entity(entity).insert((
            Sprite::from_color(
                Color::srgb(template.color[0], template.color[1], template.color[2]),
                Vec2::splat(template.size),
            ),
            Transform::default(),
        ));
    }
}

fn update_delta_seconds(time: Res<Time>, mut delta: ResMut<DeltaSeconds>) {
    delta.0 = time.delta_secs();
}

fn player_input_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    local_player: Res<LocalPlayer>,
    mut move_input: MessageWriter<MoveInput>,
    mut attack_input: MessageWriter<AttackInput>,
    mut revive_input: MessageWriter<ReviveInput>,
) {
    if local_player.0.is_none() {
        return;
    }

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
        attack_input.write(AttackInput);
    }

    revive_input.write(ReviveInput {
        held: keyboard.pressed(KeyCode::KeyF),
    });
}

fn sync_transform_system(mut query: Query<(&Position, &mut Transform)>) {
    for (position, mut transform) in &mut query {
        transform.translation.x = position.x;
        transform.translation.y = position.y;
    }
}

/// Query filter matching any party member (local or remote) with a position.
type PartyPositions<'w, 's> =
    Query<'w, 's, &'static Position, Or<(With<Player>, With<RemotePlayer>)>>;

/// Returns the party's centroid and the greatest distance from it to any
/// party member, or `None` if no party member positions exist yet. Shared by
/// the camera and leash-indicator systems so they don't each recompute it.
fn party_centroid_and_spread(party: &PartyPositions) -> Option<(Vec2, f32)> {
    let count = party.iter().count();
    if count == 0 {
        return None;
    }

    let (sum_x, sum_y) = party.iter().fold((0.0, 0.0), |(sx, sy), position| {
        (sx + position.x, sy + position.y)
    });
    let centroid = Vec2::new(sum_x / count as f32, sum_y / count as f32);
    let max_radius = party
        .iter()
        .map(|position| Vec2::new(position.x, position.y).distance(centroid))
        .fold(0.0_f32, f32::max);

    Some((centroid, max_radius))
}

/// Centers the camera on the party's centroid and zooms out as the party
/// spreads apart — see DESIGN.md's Camera & movement section. `max_radius`
/// is doubled to approximate the party's full spread (diameter) from its
/// centroid distance (radius), matching the leash boundary enforced
/// server-side in `game_core::leash_system`.
fn party_camera_system(
    mut camera: Query<(&mut Transform, &mut Projection), With<Camera2d>>,
    party: PartyPositions,
) {
    let Some((centroid, max_radius)) = party_centroid_and_spread(&party) else {
        return;
    };
    let Ok((mut transform, mut projection)) = camera.single_mut() else {
        return;
    };

    transform.translation.x = centroid.x;
    transform.translation.y = centroid.y;

    let spread_ratio = (max_radius * 2.0 / LEASH_DISTANCE).clamp(0.0, 1.0);
    if let Projection::Orthographic(ortho) = &mut *projection {
        ortho.scale = MIN_ZOOM + (MAX_ZOOM - MIN_ZOOM) * spread_ratio;
    }
}

type PartySprites<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static mut Sprite, Has<Downed>),
    Or<(With<Player>, With<RemotePlayer>)>,
>;

/// Fully recomputes every party member's sprite color each frame, rather
/// than overlaying a tint on top of whatever color was there before — that
/// overlay approach can't un-tint a `RemotePlayer` once `Downed` is
/// removed (ally-revive), since nothing else runs every frame to restore
/// their base color. `Downed` wins over the leash-warning tint; only the
/// local player gets a leash-warning tint at all (the boundary itself is
/// enforced server-side, see DESIGN.md's Camera & movement section — this
/// is cosmetic feedback, not the real HUD indicator planned for M8).
fn player_appearance_system(
    local_player: Res<LocalPlayer>,
    party: PartyPositions,
    mut sprites: PartySprites,
) {
    let centroid = party_centroid_and_spread(&party).map(|(centroid, _)| centroid);
    let warning_threshold = (LEASH_DISTANCE / 2.0) * LEASH_WARNING_RATIO;

    for (entity, mut sprite, downed) in &mut sprites {
        sprite.color = if downed {
            DOWNED_COLOR
        } else if Some(entity) == local_player.0 {
            let near_leash_limit =
                centroid
                    .zip(party.get(entity).ok())
                    .is_some_and(|(centroid, position)| {
                        Vec2::new(position.x, position.y).distance(centroid) >= warning_threshold
                    });
            if near_leash_limit {
                PLAYER_LEASH_WARNING_COLOR
            } else {
                PLAYER_COLOR
            }
        } else {
            REMOTE_PLAYER_COLOR
        };
    }
}
