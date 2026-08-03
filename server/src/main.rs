use std::net::{Ipv4Addr, UdpSocket};
use std::path::Path;
use std::time::{Duration, SystemTime};

use avian2d::math::Vector;
use avian2d::prelude::{
    Collider, Friction, Gravity, LinearVelocity, LockedAxes, PhysicsPlugins,
    Position as PhysicsPosition, RigidBody,
};
use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::{
    netcode::{NetcodeServerTransport, ServerAuthentication, ServerConfig},
    renet::ConnectionConfig,
    RenetChannelsExt, RenetServer, RepliconRenetPlugins,
};
use content::{load_all_enemy_templates, spawn_enemy};
use game_core::combat::{
    attack_system, death_system, tick_attack_timers, AttackRequested, AttackTimer, CombatStats,
    DamageType, Health, MeleeAttack,
};
use game_core::enemy::ai_system;
use game_core::movement::{leash_system, movement_system};
use game_core::{DeltaSeconds, MoveSpeed, Player, Position};
use protocol::{AttackInput, MoveInput, NetworkPlugin, PROTOCOL_ID, SERVER_PORT};

const PLAYER_SPEED: f32 = 200.0;
const PLAYER_COLLIDER_RADIUS: f32 = 16.0;
const PLAYER_MAX_HEALTH: f32 = 100.0;
const PLAYER_ATTACK_RANGE: f32 = 60.0;
const PLAYER_ATTACK_DAMAGE: f32 = 15.0;
const PLAYER_ATTACK_COOLDOWN: f32 = 0.4;
// A normal weapon strike — not yet one of the "christian" holy/radiant
// types introduced by later enemy tiers (see DESIGN.md's Damage & faction
// system and Enemy tiering sections).
const PLAYER_ATTACK_DAMAGE_TYPE: &str = "primal";
const PLAYER_CRIT_CHANCE: f32 = 0.1;
const PLAYER_CRIT_MULTIPLIER: f32 = 1.5;
const MAX_CLIENTS: usize = 2;
const TICK_RATE: f64 = 60.0;

const MAP_PATH: &str = "assets/maps/valley.tmx";
const ENEMY_TEMPLATES_DIR: &str = "assets/enemies";
// Enemies spawn in the map's bottom open field (rows 12-14, well clear of
// the mountain band and the player's top-left spawn point) — see
// assets/maps/valley.tmx and spawn_map_colliders' coordinate convention.
const ENEMY_SPAWN_BASE: Position = Position {
    x: 150.0,
    y: -420.0,
};
const ENEMY_SPAWN_SPACING: f32 = 150.0;

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
        .add_plugins(PhysicsPlugins::default())
        // Top-down game: no downward pull, movement is fully player/AI-driven.
        .insert_resource(Gravity(Vector::ZERO))
        .init_resource::<DeltaSeconds>()
        .add_message::<AttackRequested>()
        .add_systems(Startup, (setup, spawn_map_colliders, spawn_enemies))
        .add_systems(
            Update,
            (
                update_delta_seconds,
                apply_move_input,
                apply_attack_input,
                sync_physics_position_to_game_core,
                leash_system,
                sync_game_core_position_to_physics,
                ai_system,
                movement_system,
                tick_attack_timers,
                attack_system,
                death_system,
            )
                .chain(),
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
///
/// Movement/collision is resolved by avian2d (`RigidBody`/`Collider`/
/// `PhysicsPosition`/`LinearVelocity`), not `game_core::movement_system` —
/// see `sync_physics_position_to_game_core` and
/// `sync_game_core_position_to_physics` for how that stays in sync with the
/// replicated `game_core::Position`.
///
/// `RigidBody::Dynamic`, not `Kinematic`: avian2d (like most physics
/// engines) never lets static/other geometry constrain a `Kinematic` body's
/// own motion — a kinematic player would simply pass through the map's
/// mountain colliders. `Dynamic` bodies do get stopped/deflected by static
/// colliders while still letting us drive them directly via `LinearVelocity`
/// each tick. `LockedAxes::ROTATION_LOCKED` and `Friction::ZERO` keep a
/// directly-velocity-driven circle from spinning or sticking on contact.
fn on_client_connected(add: On<Add, ConnectedClient>, mut commands: Commands) {
    commands.entity(add.entity).insert((
        Player,
        Position { x: 0.0, y: 0.0 },
        MoveSpeed(PLAYER_SPEED),
        Health::new(PLAYER_MAX_HEALTH),
        MeleeAttack {
            range: PLAYER_ATTACK_RANGE,
            damage: PLAYER_ATTACK_DAMAGE,
            cooldown: PLAYER_ATTACK_COOLDOWN,
            damage_type: DamageType(PLAYER_ATTACK_DAMAGE_TYPE.to_string()),
        },
        CombatStats {
            crit_chance: PLAYER_CRIT_CHANCE,
            crit_multiplier: PLAYER_CRIT_MULTIPLIER,
        },
        AttackTimer(0.0),
        Replicated,
        RigidBody::Dynamic,
        Collider::circle(PLAYER_COLLIDER_RADIUS),
        LockedAxes::ROTATION_LOCKED,
        Friction::ZERO,
        PhysicsPosition(Vector::ZERO),
        LinearVelocity::default(),
    ));
}

/// Loads every enemy template and spawns one instance of each in the map's
/// bottom open field. Enemies are simulated with the plain
/// `game_core::movement` integrator (`Velocity`/`movement_system`), not
/// avian2d — they don't yet collide with map terrain, only chase/attack the
/// nearest player (see `game_core::enemy::ai_system`).
fn spawn_enemies(mut commands: Commands) {
    let templates = load_all_enemy_templates(Path::new(ENEMY_TEMPLATES_DIR))
        .unwrap_or_else(|error| panic!("failed to load enemy templates: {error}"));

    for (index, (kind, template)) in templates.iter().enumerate() {
        let position = Position {
            x: ENEMY_SPAWN_BASE.x + index as f32 * ENEMY_SPAWN_SPACING,
            y: ENEMY_SPAWN_BASE.y,
        };
        let entity = spawn_enemy(&mut commands, kind.clone(), template, position);
        commands.entity(entity).insert(Replicated);
    }
}

/// Loads the map's "collision" object layer and spawns a static avian2d
/// collider per polygon. Uses the plain `tiled` crate (not `bevy_ecs_tiled`)
/// because `bevy_ecs_tiled` hard-depends on `bevy_render` regardless of
/// features — see `CLAUDE.md`'s feature-unification note. The client
/// renders the same map via `bevy_ecs_tiled` with its map anchor set to
/// `TopLeft`, which makes `world = (tiled_x, -tiled_y)` the exact conversion
/// with no map-size-dependent offset — keeping this hand-rolled parse
/// aligned with what the client draws.
fn spawn_map_colliders(mut commands: Commands) {
    let mut loader = tiled::Loader::new();
    let map = loader
        .load_tmx_map(Path::new(MAP_PATH))
        .unwrap_or_else(|error| panic!("failed to load map {MAP_PATH}: {error}"));

    for layer in map.layers() {
        let tiled::LayerType::Objects(object_layer) = layer.layer_type() else {
            continue;
        };
        for object in object_layer.objects() {
            let tiled::ObjectShape::Polygon { points } = &object.shape else {
                continue;
            };
            let world_points: Vec<Vector> = points
                .iter()
                .map(|(x, y)| Vector::new(object.x + x, -(object.y + y)))
                .collect();
            let Some(collider) = Collider::convex_hull(world_points) else {
                continue;
            };
            commands.spawn((RigidBody::Static, collider));
        }
    }
}

fn update_delta_seconds(time: Res<Time>, mut delta: ResMut<DeltaSeconds>) {
    delta.0 = time.delta_secs();
}

fn apply_move_input(
    mut inputs: MessageReader<FromClient<MoveInput>>,
    mut players: Query<(&MoveSpeed, &mut LinearVelocity), With<Player>>,
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

/// Turns a client's `AttackInput` message into an `AttackRequested` event for
/// their own player entity — `attack_system` resolves range/cooldown/target/
/// damage from there, same as it does for enemy-initiated attacks from
/// `ai_system`.
fn apply_attack_input(
    mut inputs: MessageReader<FromClient<AttackInput>>,
    mut attack_events: MessageWriter<AttackRequested>,
) {
    for input in inputs.read() {
        let Some(entity) = input.client_id.entity() else {
            continue;
        };
        attack_events.write(AttackRequested { attacker: entity });
    }
}

/// Copies avian2d's resolved position into the replicated `game_core::Position`
/// each tick, after the physics step has already run (avian2d schedules its
/// solver in `FixedUpdate`, which runs before `Update` in Bevy's schedule
/// order) — see `on_client_connected`'s doc comment.
fn sync_physics_position_to_game_core(
    mut players: Query<(&PhysicsPosition, &mut Position), With<Player>>,
) {
    for (physics_position, mut position) in &mut players {
        position.x = physics_position.x;
        position.y = physics_position.y;
    }
}

/// Writes `leash_system`'s clamped `game_core::Position` back into avian2d's
/// own `PhysicsPosition`, so next tick's physics step starts from the
/// clamped position rather than silently un-clamping it — see
/// `on_client_connected`'s doc comment.
fn sync_game_core_position_to_physics(
    mut players: Query<(&Position, &mut PhysicsPosition), With<Player>>,
) {
    for (position, mut physics_position) in &mut players {
        physics_position.x = position.x;
        physics_position.y = position.y;
    }
}
