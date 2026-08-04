mod persistence;

use std::collections::HashMap;
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
use bevy_replicon::shared::backend::connected_client::NetworkId;
use bevy_replicon_renet::{
    netcode::{NetcodeServerTransport, ServerAuthentication, ServerConfig},
    renet::ConnectionConfig,
    RenetChannelsExt, RenetServer, RepliconRenetPlugins,
};
use content::{
    load_all_enemy_templates, load_all_item_templates, load_all_rune_templates,
    load_all_skill_templates, spawn_enemy,
};
use game_core::combat::{
    attack_system, death_system, tick_attack_timers, AttackRequested, AttackTimer, CombatStats,
    DamageType, Health, MeleeAttack,
};
use game_core::enemy::ai_system;
use game_core::movement::leash_system;
use game_core::{
    allocate_stat_point, apply_death_xp_penalty, equip_item, learn_skill, pickup_loot,
    reset_xp_on_full_wipe, revive_system, skill_cast_system, socket_rune, tick_od_regen,
    tick_skill_cooldowns, tick_status_effects, unequip_item, unsocket_rune, ActiveEffects,
    DeltaSeconds, Downed, EffectDefinition, EffectKind, EffectTarget, Enemy, Equipment, Facing,
    Inventory, ItemDrop, ItemLibrary, KnownSkills, Level, MoveSpeed, Od, Player, Position,
    Reviving, RuneInventory, RuneLibrary, SkillCastRequested, SkillCooldowns, SkillLibrary,
    StackMode, Stat, Stats, Stunned, UnspentSkillPoints, UnspentStatPoints, Velocity,
};
use protocol::{
    AllocateStatPointInput, AttackInput, CastSkillInput, ConnectAuth, EquipItemInput,
    LearnSkillInput, MoveInput, NetworkPlugin, PickupItemInput, ReviveInput, SocketRuneInput,
    UnequipItemInput, UnsocketRuneInput, PROTOCOL_ID, SERVER_PORT,
};

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
// "Fury": a self-buff that procs on a landed hit, stacking independently
// (see MECHANICS.md's Combat section) — consecutive hits build separate
// stacking crit-chance bonuses, self-limited by attack cooldown × duration
// rather than growing unbounded.
const PLAYER_FURY_ID: &str = "fury";
const PLAYER_FURY_CRIT_CHANCE_BONUS: f32 = 0.05;
const PLAYER_FURY_DURATION_SECS: f32 = 3.0;
const PLAYER_OD_MAX: f32 = 100.0;
const PLAYER_OD_REGEN_RATE: f32 = 5.0;
// How close a player needs to be to a dropped item/rune to pick it up —
// tuned loosely against PLAYER_ATTACK_RANGE, not meant to require pixel
// precision.
const PICKUP_RANGE: f32 = 50.0;
const MAX_CLIENTS: usize = 2;
const TICK_RATE: f64 = 60.0;

const MAP_PATH: &str = "assets/maps/valley.tmx";
const SKILL_TEMPLATES_DIR: &str = "assets/spells";
const ENEMY_TEMPLATES_DIR: &str = "assets/enemies";
const ITEM_TEMPLATES_DIR: &str = "assets/items";
const RUNE_TEMPLATES_DIR: &str = "assets/runes";
// Enemies spawn in the map's bottom open field (rows 12-14, well clear of
// the mountain band and the player's top-left spawn point) — see
// assets/maps/valley.tmx and spawn_map_colliders' coordinate convention.
const ENEMY_SPAWN_BASE: Position = Position {
    x: 150.0,
    y: -420.0,
};
const ENEMY_SPAWN_SPACING: f32 = 150.0;

/// Save files live under `saves/<game_id>/` — see `persistence`. One server
/// process is one game (see DECISIONS.md), so this is a directory
/// namespace, not a lookup across multiple simultaneous games.
const SAVES_DIR: &str = "saves";

/// Which save directory this server instance reads/writes — the first CLI
/// argument, or `"default"` if none given (`cargo run -p server --
/// my_game`). Purely a server-side launch concern: the client never sees
/// or supplies this, since one server process already is one game from a
/// connecting client's point of view (see DECISIONS.md's identity-model
/// entry for why a client-facing "game ID" was scoped out of this pass).
#[derive(Resource, Clone)]
struct GameId(String);

/// The game's shared password, checked against a connecting client's
/// `ConnectAuth::game_password`. `None` until the very first successful
/// connection, which claims whatever password it supplied as canonical
/// (see `on_client_connected`).
#[derive(Resource, Default)]
struct GamePassword(Option<String>);

/// Currently-connected character IDs, keyed to their entity — used to
/// reject a second simultaneous connection for the same character (see
/// `on_client_connected`) and to look up who to save on disconnect (see
/// `on_character_disconnected`).
#[derive(Resource, Default)]
struct ActiveCharacters(HashMap<u128, Entity>);

/// This connection's persistent character identity — server-only, not
/// replicated (no one else needs it). See DECISIONS.md's identity-model
/// entry: a character is scoped to this one game, not portable across
/// different games.
#[derive(Component, Debug, Clone, Copy)]
struct CharacterId(u128);

fn main() {
    let game_id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "default".to_string());
    // Plain `println!`, not `info!` — this runs before `App::new()` even
    // adds `LogPlugin`, so `tracing`'s global subscriber doesn't exist yet
    // and an `info!` call here would be silently dropped.
    println!("using game id {game_id:?} (saves/{game_id}/)");

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
        .insert_resource(GameId(game_id))
        .init_resource::<DeltaSeconds>()
        .init_resource::<ActiveCharacters>()
        .add_message::<AttackRequested>()
        .add_message::<SkillCastRequested>()
        .add_systems(
            Startup,
            (
                load_game_password,
                load_skills,
                load_items,
                setup,
                spawn_map_colliders,
                spawn_enemies,
            ),
        )
        .add_systems(
            Update,
            (
                // Split into chained groups, not one flat tuple — Bevy's
                // `IntoSystemConfigs` tuple impl is only generated up to a
                // fixed arity, and this pass pushed the total past it.
                // Nesting still preserves the exact same overall ordering:
                // the outer `.chain()` runs each group to completion before
                // the next one starts, same as one long chain would.
                (
                    update_delta_seconds,
                    apply_move_input,
                    apply_attack_input,
                    apply_skill_cast_input,
                    apply_revive_input,
                    freeze_incapacitated_players,
                    sync_physics_position_to_game_core,
                    leash_system,
                    sync_game_core_position_to_physics,
                    ai_system,
                )
                    .chain(),
                (
                    sync_enemy_velocity_to_physics,
                    tick_attack_timers,
                    tick_od_regen,
                    tick_skill_cooldowns,
                    attack_system,
                    skill_cast_system,
                    tick_status_effects,
                    death_system,
                    apply_death_xp_penalty,
                    reset_xp_on_full_wipe,
                    revive_system,
                )
                    .chain(),
                (
                    tag_item_drops_for_replication,
                    apply_pickup_input,
                    apply_equip_input,
                    apply_unequip_input,
                    apply_socket_rune_input,
                    apply_unsocket_rune_input,
                    apply_allocate_stat_point_input,
                    apply_learn_skill_input,
                )
                    .chain(),
            )
                .chain(),
        )
        .add_observer(on_client_connected)
        .add_observer(on_character_disconnected)
        .add_observer(on_player_downed)
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

/// Loads this game's existing password from `saves/<game_id>/game.ron` at
/// startup, if the file exists — a brand-new game (no file yet) leaves
/// `GamePassword` at its default `None`, claimed by whoever connects first
/// (see `on_client_connected`). A corrupt file panics here, at Startup
/// before anyone's connected, matching the same "malformed content fails
/// loudly" convention `spawn_enemies`/`spawn_map_colliders` already use —
/// nothing mid-session is lost by refusing to start.
fn load_game_password(mut commands: Commands, game_id: Res<GameId>) {
    let existing = persistence::load_game_save(Path::new(SAVES_DIR), &game_id.0)
        .unwrap_or_else(|error| panic!("failed to load game save: {error}"));
    commands.insert_resource(GamePassword(existing.map(|save| save.password)));
}

/// Loads every skill template into a `Res<SkillLibrary>`, looked up by id
/// at cast time (see `game_core::skill_cast_system`) — unlike enemy
/// templates, a skill's definition isn't spawned onto an entity once, so
/// it needs to stay around as a resource rather than being consumed at
/// Startup. A malformed file panics here, same "fail loudly before anyone
/// connects" convention as `spawn_enemies`/`spawn_map_colliders`.
fn load_skills(mut commands: Commands) {
    let templates = load_all_skill_templates(Path::new(SKILL_TEMPLATES_DIR))
        .unwrap_or_else(|error| panic!("failed to load skill templates: {error}"));
    let library = templates
        .into_iter()
        .map(|(id, template)| (id, template.into_definition()))
        .collect();
    commands.insert_resource(SkillLibrary(library));
}

/// Loads item and rune templates into `Res<ItemLibrary>`/`Res<RuneLibrary>`
/// — same "fail loudly before anyone connects" convention as
/// `load_skills`/`spawn_enemies`. Both libraries are needed by
/// `combat::attack_system`/`death_system` (equipment stat bonuses, loot
/// rolls) even though no enemy template references an item/rune that
/// doesn't exist — a bad reference would only surface as "nothing dropped"
/// rather than a panic, since `roll_loot` treats an unknown template key as
/// zero sockets rather than failing (see its doc comment).
fn load_items(mut commands: Commands) {
    let item_templates = load_all_item_templates(Path::new(ITEM_TEMPLATES_DIR))
        .unwrap_or_else(|error| panic!("failed to load item templates: {error}"));
    let items = item_templates
        .into_iter()
        .map(|(id, template)| (id, template.into_definition()))
        .collect();
    commands.insert_resource(ItemLibrary(items));

    let rune_templates = load_all_rune_templates(Path::new(RUNE_TEMPLATES_DIR))
        .unwrap_or_else(|error| panic!("failed to load rune templates: {error}"));
    let runes = rune_templates
        .into_iter()
        .map(|(id, template)| (id, template.into_definition()))
        .collect();
    commands.insert_resource(RuneLibrary(runes));
}

/// Every connected client is represented as an entity with `ConnectedClient`;
/// we attach the player's gameplay components directly to that same entity
/// rather than tracking a separate client-to-player mapping.
///
/// Before any gameplay setup, validates the `ConnectAuth` payload carried
/// in netcode's connection-time `user_data` (see `protocol::ConnectAuth`):
/// the game password must match (or this is the very first connection
/// ever, which claims the supplied password as canonical), the character
/// must not already be connected (`ActiveCharacters`), and if a save
/// already exists for that character its password must match too. Any
/// failure disconnects the client and returns before any gameplay
/// component is inserted — `bevy_replicon_renet` despawns the
/// (`ConnectedClient`-only) entity itself once the disconnect is
/// processed (verified in its source), so there's nothing to clean up
/// here.
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
// Eight params is inherent to what this system does (connection lifecycle,
// auth validation, and persistence each need their own resource/query) —
// splitting it up would mean passing most of these straight through to a
// helper anyway, not reducing real complexity.
#[allow(clippy::too_many_arguments)]
fn on_client_connected(
    add: On<Add, ConnectedClient>,
    mut commands: Commands,
    network_ids: Query<&NetworkId>,
    transport: Res<NetcodeServerTransport>,
    mut server: ResMut<RenetServer>,
    game_id: Res<GameId>,
    mut game_password: ResMut<GamePassword>,
    mut active_characters: ResMut<ActiveCharacters>,
) {
    let entity = add.entity;
    let Ok(&network_id) = network_ids.get(entity) else {
        return;
    };
    let client_id = network_id.get();
    let saves_dir = Path::new(SAVES_DIR);

    let Some(user_data) = transport.user_data(client_id) else {
        warn!("rejecting client {client_id}: no auth data supplied");
        server.disconnect(client_id);
        return;
    };
    let Ok(auth) = ConnectAuth::decode(&user_data) else {
        warn!("rejecting client {client_id}: malformed auth data");
        server.disconnect(client_id);
        return;
    };

    match &game_password.0 {
        Some(expected) if *expected != auth.game_password => {
            warn!("rejecting client {client_id}: wrong game password");
            server.disconnect(client_id);
            return;
        }
        Some(_) => {}
        None => {
            let save = persistence::GameSave {
                password: auth.game_password.clone(),
            };
            if let Err(error) = persistence::save_game_save(saves_dir, &game_id.0, &save) {
                error!("rejecting client {client_id}: failed to create new game save: {error}");
                server.disconnect(client_id);
                return;
            }
            game_password.0 = Some(auth.game_password.clone());
        }
    }

    if active_characters.0.contains_key(&auth.character_id) {
        warn!(
            "rejecting client {client_id}: character {} is already connected",
            auth.character_id
        );
        server.disconnect(client_id);
        return;
    }

    let (level, stats, points, known_skills, skill_points, inventory, equipment, runes) =
        match persistence::load_character_save(saves_dir, &game_id.0, auth.character_id) {
            Ok(Some(save)) => {
                if save.password != auth.character_password {
                    warn!("rejecting client {client_id}: wrong character password");
                    server.disconnect(client_id);
                    return;
                }
                (
                    save.level,
                    save.stats,
                    save.points,
                    save.known_skills,
                    save.skill_points,
                    save.inventory,
                    save.equipment,
                    save.runes,
                )
            }
            Ok(None) => {
                let save = persistence::CharacterSave {
                    password: auth.character_password.clone(),
                    level: Level::default(),
                    stats: Stats::default(),
                    points: UnspentStatPoints::default(),
                    known_skills: KnownSkills::default(),
                    skill_points: UnspentSkillPoints::default(),
                    inventory: Inventory::default(),
                    equipment: Equipment::default(),
                    runes: RuneInventory::default(),
                };
                if let Err(error) = persistence::save_character_save(
                    saves_dir,
                    &game_id.0,
                    auth.character_id,
                    &save,
                ) {
                    error!(
                        "rejecting client {client_id}: failed to create new character save: {error}"
                    );
                    server.disconnect(client_id);
                    return;
                }
                (
                    save.level,
                    save.stats,
                    save.points,
                    save.known_skills,
                    save.skill_points,
                    save.inventory,
                    save.equipment,
                    save.runes,
                )
            }
            Err(error) => {
                error!("rejecting client {client_id}: failed to load character save: {error}");
                server.disconnect(client_id);
                return;
            }
        };

    active_characters.0.insert(auth.character_id, entity);

    commands.entity(entity).insert((
        Player,
        CharacterId(auth.character_id),
        level,
        stats,
        points,
        (
            known_skills,
            skill_points,
            SkillCooldowns::default(),
            Od::new(PLAYER_OD_MAX, PLAYER_OD_REGEN_RATE),
            inventory,
            equipment,
            runes,
        ),
        Position { x: 0.0, y: 0.0 },
        Facing::default(),
        MoveSpeed(PLAYER_SPEED),
        Health::new(PLAYER_MAX_HEALTH),
        MeleeAttack {
            range: PLAYER_ATTACK_RANGE,
            damage: PLAYER_ATTACK_DAMAGE,
            cooldown: PLAYER_ATTACK_COOLDOWN,
            damage_type: DamageType(PLAYER_ATTACK_DAMAGE_TYPE.to_string()),
            effects: vec![EffectDefinition {
                id: PLAYER_FURY_ID.to_string(),
                kind: EffectKind::StatModifier {
                    stat: Stat::CritChance,
                },
                duration: PLAYER_FURY_DURATION_SECS,
                magnitude: PLAYER_FURY_CRIT_CHANCE_BONUS,
                stack_mode: StackMode::Independent,
                applies_to: EffectTarget::Attacker,
                chance: 1.0,
            }],
        },
        CombatStats {
            crit_chance: PLAYER_CRIT_CHANCE,
            crit_multiplier: PLAYER_CRIT_MULTIPLIER,
        },
        AttackTimer(0.0),
        ActiveEffects::default(),
        (
            Replicated,
            RigidBody::Dynamic,
            Collider::circle(PLAYER_COLLIDER_RADIUS),
            LockedAxes::ROTATION_LOCKED,
            Friction::ZERO,
            PhysicsPosition(Vector::ZERO),
            LinearVelocity::default(),
        ),
    ));
}

/// Everything persisted to a `CharacterSave` on disconnect — see
/// `on_character_disconnected`.
type PersistedCharacterData<'w, 's> = Query<
    'w,
    's,
    (
        &'static CharacterId,
        &'static Level,
        &'static Stats,
        &'static UnspentStatPoints,
        &'static KnownSkills,
        &'static UnspentSkillPoints,
        &'static Inventory,
        &'static Equipment,
        &'static RuneInventory,
    ),
>;

/// Persists a character's current `Level`/`Stats`/`UnspentStatPoints` back
/// to its save file the moment they disconnect. Fires on `Remove` (which
/// runs *before* the component is actually gone, so the rest of the
/// entity's data is still readable), not by reacting to
/// `RenetServerEvent(ServerEvent::ClientDisconnected)` directly — that
/// event is also how `bevy_replicon_renet` itself decides to despawn the
/// entity (verified in its source), and relying on two separate observers
/// for the same custom event to run in a particular order isn't a
/// guarantee Bevy actually makes; a component-removal hook on the entity
/// being despawned is.
///
/// A connection rejected in `on_client_connected` (disconnected before any
/// gameplay component was inserted) has no `CharacterId` here and is
/// silently skipped — there was never anything to save.
fn on_character_disconnected(
    remove: On<Remove, ConnectedClient>,
    mut active_characters: ResMut<ActiveCharacters>,
    game_id: Res<GameId>,
    characters: PersistedCharacterData,
) {
    let Ok((
        character_id,
        level,
        stats,
        points,
        known_skills,
        skill_points,
        inventory,
        equipment,
        runes,
    )) = characters.get(remove.entity)
    else {
        return;
    };
    active_characters.0.remove(&character_id.0);

    let saves_dir = Path::new(SAVES_DIR);
    let password = match persistence::load_character_save(saves_dir, &game_id.0, character_id.0) {
        Ok(Some(save)) => save.password,
        Ok(None) => {
            error!(
                "character {} disconnected with no save file to preserve a password for",
                character_id.0
            );
            return;
        }
        Err(error) => {
            error!(
                "failed to reload character {} save before writing: {error}",
                character_id.0
            );
            return;
        }
    };

    let save = persistence::CharacterSave {
        password,
        level: *level,
        stats: *stats,
        points: *points,
        known_skills: known_skills.clone(),
        skill_points: *skill_points,
        inventory: inventory.clone(),
        equipment: equipment.clone(),
        runes: runes.clone(),
    };
    if let Err(error) =
        persistence::save_character_save(saves_dir, &game_id.0, character_id.0, &save)
    {
        error!(
            "failed to save character {} on disconnect: {error}",
            character_id.0
        );
    }
}

/// A player who dies mid-move would otherwise keep sliding under whatever
/// `LinearVelocity` they last had — `apply_move_input` stops issuing new
/// velocity for a downed player (see below) but never zeroes out what's
/// already there. This only handles the instant of going down; see
/// `freeze_incapacitated_players` for why a downed player's velocity also
/// needs zeroing on every later tick, not just this one.
fn on_player_downed(add: On<Add, Downed>, mut velocities: Query<&mut LinearVelocity>) {
    if let Ok(mut velocity) = velocities.get_mut(add.entity) {
        *velocity = LinearVelocity::ZERO;
    }
}

/// A downed player keeps a solid `Collider`/`RigidBody::Dynamic` (see
/// `MECHANICS.md`'s downed-state section — still physically present, not
/// incorporeal), so another player or an enemy bumping into them imparts a
/// fresh physics impulse into their `LinearVelocity` on every such
/// collision, not just at the moment they went down. Nothing else ever
/// issues a downed *or stunned* player new velocity (`apply_move_input`
/// skips both via `Without<Downed>, Without<Stunned>`), so left alone that
/// impulse would integrate into unbounded drift instead of a single
/// bounded nudge — found via live testing, not a hypothetical. Re-zeroing
/// every tick stops it from compounding tick over tick; it can't undo the
/// current tick's already-resolved physics step, so a small one-tick nudge
/// on contact is expected and fine, just not a runaway one.
type Incapacitated = Or<(With<Downed>, With<Stunned>)>;

fn freeze_incapacitated_players(mut players: Query<&mut LinearVelocity, Incapacitated>) {
    for mut velocity in &mut players {
        *velocity = LinearVelocity::ZERO;
    }
}

/// Loads every enemy template and spawns one instance of each in the map's
/// bottom open field. `game_core::enemy::ai_system` decides each enemy's
/// desired `Velocity` (chase/idle/attack); `avian2d` resolves the actual
/// movement and collision against terrain, players, and other enemies, same
/// as `on_client_connected`'s player setup — see
/// `sync_enemy_velocity_to_physics` for how `Velocity` gets translated into
/// `LinearVelocity` each tick.
///
/// No explicit `Mass`/`ColliderDensity`: avian2d auto-computes mass from a
/// `Collider`'s shape area × density (default `1.0`), so sizing the collider
/// from the same `template.size` already used for the sprite gives bigger
/// enemies proportionally more mass "for free" — a big enemy barely budges
/// when a player bumps it, a small one gets shoved more easily, with no new
/// content field needed.
fn spawn_enemies(mut commands: Commands) {
    let templates = load_all_enemy_templates(Path::new(ENEMY_TEMPLATES_DIR))
        .unwrap_or_else(|error| panic!("failed to load enemy templates: {error}"));

    for (index, (kind, template)) in templates.iter().enumerate() {
        let position = Position {
            x: ENEMY_SPAWN_BASE.x + index as f32 * ENEMY_SPAWN_SPACING,
            y: ENEMY_SPAWN_BASE.y,
        };
        let entity = spawn_enemy(&mut commands, kind.clone(), template, position);
        commands.entity(entity).insert((
            Replicated,
            RigidBody::Dynamic,
            Collider::circle(template.size / 2.0),
            LockedAxes::ROTATION_LOCKED,
            Friction::ZERO,
            PhysicsPosition(Vector::new(position.x, position.y)),
            LinearVelocity::default(),
        ));
    }
}

/// Tags a freshly-spawned `ItemDrop` (see `combat::death_system`'s loot
/// roll) with `Replicated` — `game_core` has no `bevy_replicon` dependency
/// (see `CLAUDE.md`'s crate boundaries), so it can't insert this marker
/// itself the way `spawn_enemies`/`on_client_connected` do inline right
/// after their own spawn call. A one-tick delay before a drop becomes
/// network-visible is imperceptible and not worth avoiding.
fn tag_item_drops_for_replication(mut commands: Commands, drops: Query<Entity, Added<ItemDrop>>) {
    for entity in &drops {
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

/// A downed or stunned player is out of action and ignores move input — see
/// `game_core::combat::attack_system`'s doc comment for the same rule
/// applied to attacking.
type MovablePlayers<'w, 's> = Query<
    'w,
    's,
    (
        &'static MoveSpeed,
        &'static mut LinearVelocity,
        &'static mut Facing,
        Option<&'static Equipment>,
        Option<&'static Stats>,
    ),
    (With<Player>, Without<Downed>, Without<Stunned>),
>;

/// A socketed move-speed rune, and a manually-allocated `Stats` point spend,
/// both add their bonus on top of the base `MoveSpeed`, computed fresh here
/// rather than mutating the base component — same "never bake a buff into
/// the base stat" principle as `combat::attack_system`'s equipment/level
/// crit bonuses.
fn apply_move_input(
    mut inputs: MessageReader<FromClient<MoveInput>>,
    mut players: MovablePlayers,
    runes: Res<RuneLibrary>,
) {
    for input in inputs.read() {
        let Some(entity) = input.client_id.entity() else {
            continue;
        };
        let Ok((speed, mut velocity, mut facing, equipment, stats)) = players.get_mut(entity)
        else {
            continue;
        };
        let equipment_bonus = equipment
            .map(|equipment| equipment.stat_bonus(Stat::MoveSpeed, &runes))
            .unwrap_or(0.0);
        let level_bonus = stats.map(|stats| stats.bonus_move_speed).unwrap_or(0.0);
        let effective_speed = speed.0 + equipment_bonus + level_bonus;
        velocity.x = input.x * effective_speed;
        velocity.y = input.y * effective_speed;
        facing.update_from_direction(input.x, input.y);
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

/// Turns a client's `CastSkillInput` into a `SkillCastRequested` event for
/// their own player entity — `skill_cast_system` resolves whether the skill
/// is actually known, off cooldown, and affordable from there, same
/// division of labor as `apply_attack_input`/`attack_system`.
fn apply_skill_cast_input(
    mut inputs: MessageReader<FromClient<CastSkillInput>>,
    mut cast_events: MessageWriter<SkillCastRequested>,
) {
    for input in inputs.read() {
        let Some(entity) = input.client_id.entity() else {
            continue;
        };
        cast_events.write(SkillCastRequested {
            caster: entity,
            skill_id: input.skill_id.clone(),
        });
    }
}

/// Turns a client's `ReviveInput` into a `Reviving` marker on their own
/// player entity, held for as long as the client keeps sending `held: true`
/// — `revive_system` (in `game_core`) does the actual range/progress/
/// completion resolution from there, same division of labor as
/// `apply_attack_input`/`attack_system`.
fn apply_revive_input(mut inputs: MessageReader<FromClient<ReviveInput>>, mut commands: Commands) {
    for input in inputs.read() {
        let Some(entity) = input.client_id.entity() else {
            continue;
        };
        if input.held {
            commands.entity(entity).insert(Reviving);
        } else {
            commands.entity(entity).remove::<Reviving>();
        }
    }
}

/// Turns a client's `PickupItemInput` into a pickup of the nearest
/// `ItemDrop` within `PICKUP_RANGE` — a deliberate button-press action, not
/// automatic walk-over pickup (see `protocol::PickupItemInput`'s doc
/// comment). Merges the drop's loot into the picker's own
/// `Inventory`/`RuneInventory` via `game_core::pickup_loot`, then despawns
/// the world-visible drop entity.
fn apply_pickup_input(
    mut inputs: MessageReader<FromClient<PickupItemInput>>,
    mut players: Query<(&Position, &mut Inventory, &mut RuneInventory)>,
    drops: Query<(Entity, &Position, &ItemDrop)>,
    mut commands: Commands,
) {
    for input in inputs.read() {
        let Some(entity) = input.client_id.entity() else {
            continue;
        };
        let Ok((player_pos, mut inventory, mut runes)) = players.get_mut(entity) else {
            continue;
        };
        let nearest = drops
            .iter()
            .filter(|(_, pos, _)| player_pos.distance(pos) <= PICKUP_RANGE)
            .min_by(|(_, a, _), (_, b, _)| {
                player_pos.distance(a).total_cmp(&player_pos.distance(b))
            });
        let Some((drop_entity, _, drop)) = nearest else {
            continue;
        };
        pickup_loot(&mut inventory, &mut runes, drop.0.clone());
        commands.entity(drop_entity).despawn();
    }
}

/// Turns a client's `EquipItemInput` into `game_core::equip_item`'s
/// resolution against their own `Inventory`/`Equipment` — a no-op if the
/// index or template is invalid (see that function's doc comment).
fn apply_equip_input(
    mut inputs: MessageReader<FromClient<EquipItemInput>>,
    mut players: Query<(&mut Inventory, &mut Equipment)>,
    items: Res<ItemLibrary>,
) {
    for input in inputs.read() {
        let Some(entity) = input.client_id.entity() else {
            continue;
        };
        let Ok((mut inventory, mut equipment)) = players.get_mut(entity) else {
            continue;
        };
        equip_item(
            &mut inventory,
            &mut equipment,
            &items,
            input.inventory_index,
        );
    }
}

/// Turns a client's `UnequipItemInput` into `game_core::unequip_item`'s
/// resolution — a no-op if nothing's equipped at that slot.
fn apply_unequip_input(
    mut inputs: MessageReader<FromClient<UnequipItemInput>>,
    mut players: Query<(&mut Inventory, &mut Equipment)>,
) {
    for input in inputs.read() {
        let Some(entity) = input.client_id.entity() else {
            continue;
        };
        let Ok((mut inventory, mut equipment)) = players.get_mut(entity) else {
            continue;
        };
        unequip_item(&mut inventory, &mut equipment, input.slot);
    }
}

/// Turns a client's `SocketRuneInput` into `game_core::socket_rune`'s
/// resolution — a no-op (see that function's doc comment) for any
/// untrusted-input case: unknown rune, empty stack, missing item, bad
/// socket index, or an already-occupied socket.
fn apply_socket_rune_input(
    mut inputs: MessageReader<FromClient<SocketRuneInput>>,
    mut players: Query<(&mut Equipment, &mut RuneInventory)>,
    runes: Res<RuneLibrary>,
) {
    for input in inputs.read() {
        let Some(entity) = input.client_id.entity() else {
            continue;
        };
        let Ok((mut equipment, mut rune_inventory)) = players.get_mut(entity) else {
            continue;
        };
        socket_rune(
            &mut equipment,
            &mut rune_inventory,
            &runes,
            input.slot,
            input.socket_index,
            &input.rune_id,
        );
    }
}

/// Turns a client's `UnsocketRuneInput` into `game_core::unsocket_rune`'s
/// resolution — free and reversible (see DECISIONS.md).
fn apply_unsocket_rune_input(
    mut inputs: MessageReader<FromClient<UnsocketRuneInput>>,
    mut players: Query<(&mut Equipment, &mut RuneInventory)>,
) {
    for input in inputs.read() {
        let Some(entity) = input.client_id.entity() else {
            continue;
        };
        let Ok((mut equipment, mut rune_inventory)) = players.get_mut(entity) else {
            continue;
        };
        unsocket_rune(
            &mut equipment,
            &mut rune_inventory,
            input.slot,
            input.socket_index,
        );
    }
}

/// Turns a client's `AllocateStatPointInput` into
/// `game_core::allocate_stat_point`'s resolution — a no-op if there's no
/// unspent point (see that function's doc comment).
fn apply_allocate_stat_point_input(
    mut inputs: MessageReader<FromClient<AllocateStatPointInput>>,
    mut players: Query<(&mut UnspentStatPoints, &mut Stats)>,
) {
    for input in inputs.read() {
        let Some(entity) = input.client_id.entity() else {
            continue;
        };
        let Ok((mut unspent, mut stats)) = players.get_mut(entity) else {
            continue;
        };
        allocate_stat_point(&mut unspent, &mut stats, input.stat);
    }
}

/// Turns a client's `LearnSkillInput` into `game_core::learn_skill`'s
/// resolution — a no-op for an unknown skill id or an empty point stack
/// (see that function's doc comment).
fn apply_learn_skill_input(
    mut inputs: MessageReader<FromClient<LearnSkillInput>>,
    mut players: Query<(&mut UnspentSkillPoints, &mut KnownSkills)>,
    skills: Res<SkillLibrary>,
) {
    for input in inputs.read() {
        let Some(entity) = input.client_id.entity() else {
            continue;
        };
        let Ok((mut unspent, mut known)) = players.get_mut(entity) else {
            continue;
        };
        learn_skill(&mut unspent, &mut known, &skills, &input.skill_id);
    }
}

/// Any `avian2d` body whose `game_core::Position` needs to stay in sync with
/// physics — players and enemies alike.
type PhysicsBodies = Or<(With<Player>, With<Enemy>)>;

/// Copies avian2d's resolved position into the replicated `game_core::Position`
/// each tick, after the physics step has already run (avian2d schedules its
/// solver in `FixedUpdate`, which runs before `Update` in Bevy's schedule
/// order) — see `on_client_connected`'s doc comment. Covers enemies too, now
/// that they're `avian2d` bodies as well (see `spawn_enemies`).
fn sync_physics_position_to_game_core(
    mut bodies: Query<(&PhysicsPosition, &mut Position), PhysicsBodies>,
) {
    for (physics_position, mut position) in &mut bodies {
        position.x = physics_position.x;
        position.y = physics_position.y;
    }
}

/// Writes `leash_system`'s clamped `game_core::Position` back into avian2d's
/// own `PhysicsPosition`, so next tick's physics step starts from the
/// clamped position rather than silently un-clamping it — see
/// `on_client_connected`'s doc comment. Covers enemies too (a no-op for them
/// today, since nothing between the two sync calls touches enemy
/// `Position` the way `leash_system` does for players — kept symmetric with
/// `sync_physics_position_to_game_core` rather than maintaining two
/// separate systems for what's otherwise identical logic).
fn sync_game_core_position_to_physics(
    mut bodies: Query<(&Position, &mut PhysicsPosition), PhysicsBodies>,
) {
    for (position, mut physics_position) in &mut bodies {
        physics_position.x = position.x;
        physics_position.y = position.y;
    }
}

/// Translates `ai_system`'s decided `Velocity` into avian2d's `LinearVelocity`
/// for enemies, the same role `apply_move_input` plays for players (reading
/// `MoveInput` instead of an AI decision). Enemies no longer move via
/// `game_core::movement::movement_system`'s plain integrator — avian2d now
/// resolves their actual position, including collision against terrain,
/// players, and each other (see `spawn_enemies`). `movement_system` itself
/// stays in `game_core`, still tested there; nothing in this binary calls it
/// anymore.
fn sync_enemy_velocity_to_physics(
    mut enemies: Query<(&Velocity, &mut LinearVelocity), With<Enemy>>,
) {
    for (velocity, mut linear_velocity) in &mut enemies {
        linear_velocity.x = velocity.x;
        linear_velocity.y = velocity.y;
    }
}
