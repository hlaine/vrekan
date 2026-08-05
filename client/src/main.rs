use std::fs;
use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::Path;
use std::time::SystemTime;

use bevy::camera::Projection;
use bevy::gizmos::prelude::*;
use bevy::prelude::*;
use bevy_ecs_tiled::prelude::{TiledMap, TiledPlugin, TilemapAnchor};
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use bevy_replicon::prelude::*;
use bevy_replicon::shared::backend::connected_client::NetworkId;
use bevy_replicon_renet::{
    netcode::{ClientAuthentication, NetcodeClientTransport},
    renet::ConnectionConfig,
    RenetChannelsExt, RenetClient, RepliconRenetPlugins,
};
use content::{
    load_all_enemy_templates, load_all_interactable_templates, load_all_item_templates,
    load_all_rune_templates, load_all_vendor_templates, EnemyTemplate, InteractableTemplate,
    ItemTemplate, RuneTemplate, VendorTemplate,
};
use game_core::movement::Position;
use game_core::player::Player;
use game_core::{
    nearest_interactable_in_range, socketed_item_sell_value, xp_required, Currency, DeltaSeconds,
    Downed, DroppedLoot, Enemy, EnemyKind, EquipSlot, Equipment, Facing, Health, Interactable,
    Inventory, Item, ItemDrop, KnownSkills, Level, Od, RuneInventory, SkillCooldowns, Stat, Stats,
    Stunned, UnspentSkillPoints, UnspentStatPoints, FORGING_PANEL_ID, LEASH_DISTANCE,
    VENDOR_PANEL_ID,
};
use protocol::{
    AllocateStatPointInput, AttackInput, BuyItemInput, CastSkillInput, ConnectAuth, EquipItemInput,
    LearnSkillInput, MoveInput, NetworkPlugin, PickupItemInput, ReviveInput, SellItemInput,
    SocketRuneInput, UnequipItemInput, UnsocketRuneInput, PROTOCOL_ID, SERVER_PORT,
};

const PLAYER_COLOR: Color = Color::srgb(0.2, 0.7, 0.3);
const REMOTE_PLAYER_COLOR: Color = Color::srgb(0.3, 0.5, 0.8);
const ITEM_DROP_COLOR: Color = Color::srgb(0.9, 0.8, 0.2);
const RUNE_DROP_COLOR: Color = Color::srgb(0.6, 0.2, 0.9);
const CURRENCY_DROP_COLOR: Color = Color::srgb(0.95, 0.75, 0.1);
const DROP_SPRITE_SIZE: f32 = 16.0;

// Placeholder appearance for a replicated `Interactable` (blacksmith,
// runestone) — a distinct blue-violet square, not real art. Larger than a
// dropped item/rune's sprite since these are meant to read as world
// fixtures (NPCs, objects), not loose pickups.
const INTERACTABLE_COLOR: Color = Color::srgb(0.4, 0.4, 0.9);
const INTERACTABLE_SPRITE_SIZE: f32 = 24.0;

/// Fixed hotkey-to-skill-id mapping — a stand-in for the real skill-tree
/// UI (M8), not a protocol concept: `CastSkillInput` carries the skill's
/// content-file id directly (see its doc comment), so this table is purely
/// a client-side input convenience that a future UI replaces with actual
/// clicks, not something the server or wire format needs to know about.
const SKILL_HOTKEYS: [(KeyCode, &str); 3] = [
    (KeyCode::Digit1, "power_strike"),
    (KeyCode::Digit2, "aoe_burst"),
    (KeyCode::Digit3, "berserk"),
];

/// Picks up the nearest dropped item/rune in range — a deliberate button
/// press, not automatic walk-over pickup (see `protocol::PickupItemInput`).
/// Gated behind the input-focus guard (see `player_input_system`) — not
/// one of `CLAUDE.md`'s explicitly-protected combat keys.
const PICKUP_KEY: KeyCode = KeyCode::KeyE;

/// Opens/closes the inventory panel (`inventory_panel_system`) — replaces
/// M7's `F1`-`F6` equip/unequip hotkey stand-ins, which are removed now
/// that the panel exists (not kept alongside it, per `ROADMAP.md`).
const INVENTORY_TOGGLE_KEY: KeyCode = KeyCode::KeyI;

/// Opens/closes the character panel (`character_panel_system`) — stat
/// allocation and skill learning, the two gaps `UnspentStatPoints`/
/// `UnspentSkillPoints` have had since M5/M6.
const CHARACTER_TOGGLE_KEY: KeyCode = KeyCode::KeyC;

// Only used to look up appearance (color/size) for a replicated enemy by
// its `EnemyKind` — the server is what actually spawns/simulates enemies
// now (see `server/src/main.rs`'s `spawn_enemies`).
const ENEMY_TEMPLATES_DIR: &str = "assets/enemies";

// Loaded locally purely for `dialog`/`opens_panels` lookup by an
// `Interactable`'s `template_key` — same "server spawns/resolves, client
// renders" split as `ENEMY_TEMPLATES_DIR`. Read by
// `interaction_trigger_system`.
const INTERACTABLE_TEMPLATES_DIR: &str = "assets/interactables";

// Loaded locally purely for `socket_cost` display/gating in the forging
// panel — same split as `ENEMY_TEMPLATES_DIR` above; the server is what
// actually resolves/charges a socket action (`game_core::socket_rune`).
const RUNE_TEMPLATES_DIR: &str = "assets/runes";

// Loaded locally purely for `sell_value` display in the vendor panel's
// sell-price preview — same split as `ENEMY_TEMPLATES_DIR` above; the
// server is what actually resolves/credits a sell (`game_core::sell_item`).
const ITEM_TEMPLATES_DIR: &str = "assets/items";

// Loaded locally purely for buy-listing display in the vendor panel —
// same split as `ENEMY_TEMPLATES_DIR` above; the server independently
// resolves/charges a purchase (`game_core::buy_item`) against its own
// copy, never trusting what the client shows.
const VENDOR_TEMPLATES_DIR: &str = "assets/vendors";

// Relative to the assets root (loaded via AssetServer), not the filesystem
// path used for ENEMY_TEMPLATES_DIR above. Must stay in sync with the
// server's MAP_PATH — see server/src/main.rs's spawn_map_colliders doc
// comment for the anchor/coordinate convention this depends on.
const MAP_PATH: &str = "maps/valley.tmx";

// Orthographic projection scale at zero and max party spread, respectively —
// tuning constants, not derived from anything.
const MIN_ZOOM: f32 = 0.7;
const MAX_ZOOM: f32 = 1.6;

// World-space extent of the current map (20x15 tiles at 32px/tile — see
// assets/maps/valley.tmx). World x runs [0, MAP_WORLD_WIDTH]; world y runs
// [-MAP_WORLD_HEIGHT, 0] (`TilemapAnchor::TopLeft` puts the origin at the
// map's top-left corner, and the established Tiled coordinate convention
// negates y — see server's spawn_interactables doc comment). Used only to
// scale the minimap; if the map ever grows, this needs updating alongside
// it since there's no dynamic map-bounds query today.
const MAP_WORLD_WIDTH: f32 = 640.0;
const MAP_WORLD_HEIGHT: f32 = 480.0;
const MINIMAP_SIZE: f32 = 120.0;
// Mirror PLAYER_COLOR/REMOTE_PLAYER_COLOR's RGB values as egui::Color32 —
// no existing bevy::Color-to-egui::Color32 conversion helper, and these
// are placeholder dots, not worth adding one for.
const MINIMAP_PLAYER_COLOR: egui::Color32 = egui::Color32::from_rgb(51, 179, 77);
const MINIMAP_REMOTE_COLOR: egui::Color32 = egui::Color32::from_rgb(77, 128, 204);

// Fraction of the leash radius (LEASH_DISTANCE / 2) at which the local
// player's leash-limit indicator kicks in.
const LEASH_WARNING_RATIO: f32 = 0.9;
const PLAYER_LEASH_WARNING_COLOR: Color = Color::srgb(0.9, 0.15, 0.15);

// Downed is a distinct, unmissable grey — deliberately not just a darker
// shade of the leash-warning red, since a downed player and a
// near-the-leash player need to read as different situations at a glance.
const DOWNED_COLOR: Color = Color::srgb(0.5, 0.5, 0.5);

// Distinct from both DOWNED_COLOR and the leash-warning red — a stunned
// player can still be attacked/downed, so it needs to read as its own
// state at a glance, not a shade of either.
const STUNNED_COLOR: Color = Color::srgb(0.9, 0.85, 0.2);

// Placeholder facing-direction indicator — a debug-style gizmo arrow, not
// real art. Length is a fraction of the player collider's diameter (32
// units) so it reads as "pointing off the sprite," not lost inside it.
const FACING_ARROW_LENGTH: f32 = 24.0;
const FACING_ARROW_COLOR: Color = Color::WHITE;

// This client's persistent character identity, generated once and reused
// across runs — not tied to any account system (see DECISIONS.md's
// identity-model entry). Relative to CWD, matching the project's existing
// "run from repo root" convention for asset/content paths. Overridable via
// a CLI arg (`cargo run -p client -- other_id.txt`) — running two clients
// from the same directory for local co-op testing would otherwise have
// both read/generate the exact same file and collide as one character.
const CHARACTER_ID_PATH: &str = "character_id.txt";

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

/// Whether the inventory panel (`inventory_panel_system`) is currently
/// shown. Toggled by `INVENTORY_TOGGLE_KEY`; also flipped to `false` by
/// egui's own window close button, since the panel is opened via
/// `egui::Window::open(&mut ...)` on this same flag.
#[derive(Resource, Default)]
struct InventoryOpen(bool);

/// Whether the character panel (`character_panel_system`) is currently
/// shown — same `egui::Window::open`-driven toggle shape as `InventoryOpen`.
#[derive(Resource, Default)]
struct CharacterPanelOpen(bool);

/// Enemy templates loaded purely for appearance lookup (color/size) by
/// `EnemyKind` — see `init_replicated_enemies`. Simulation-relevant fields
/// (health, damage, AI ranges) only matter server-side now.
#[derive(Resource)]
struct EnemyTemplates(Vec<(String, EnemyTemplate)>);

/// Interactable templates loaded purely for `dialog`/`opens_panels` lookup
/// by an `Interactable`'s `template_key` — see `INTERACTABLE_TEMPLATES_DIR`'s
/// doc comment.
#[derive(Resource)]
struct InteractableTemplates(Vec<(String, InteractableTemplate)>);

/// Rune templates loaded purely for `socket_cost` lookup by rune id in the
/// forging panel — see `RUNE_TEMPLATES_DIR`'s doc comment.
#[derive(Resource)]
struct RuneTemplates(Vec<(String, RuneTemplate)>);

/// Item templates loaded purely for `sell_value` lookup by template key in
/// the vendor panel's sell-price preview — see `ITEM_TEMPLATES_DIR`'s doc
/// comment.
#[derive(Resource)]
struct ItemTemplates(Vec<(String, ItemTemplate)>);

/// Vendor templates loaded purely for buy-listing display — see
/// `VENDOR_TEMPLATES_DIR`'s doc comment. Looked up by the active vendor's
/// `template_key` (`InteractionPanels::vendor`), not a single global list —
/// each vendor has its own stock.
#[derive(Resource)]
struct VendorTemplates(Vec<(String, VendorTemplate)>);

/// Which panel(s) `interaction_trigger_system` has opened. Unlike M8's
/// original single-panel design, one `Interactable` can declare more than
/// one `opens_panels` capability (e.g. a blacksmith is both `"forging"`
/// and `"vendor"`, see `game_core::FORGING_PANEL_ID`/`VENDOR_PANEL_ID`),
/// so `E` opens every capability the nearest interactable has rather than
/// picking just one — three independent fields instead of an enum, since
/// any subset can be open at once. `dialog` only ever gets set when
/// neither panel applies (see `interaction_trigger_system`), matching the
/// old either-or behavior for runestone-style NPCs. `vendor` carries
/// *which* vendor's `template_key` to read its `VendorTemplate` listing
/// from — not just a bool, since which vendor matters for that lookup.
#[derive(Resource, Default)]
struct InteractionPanels {
    dialog: Option<String>,
    forging_open: bool,
    vendor: Option<String>,
}

impl InteractionPanels {
    fn any_open(&self) -> bool {
        self.dialog.is_some() || self.forging_open || self.vendor.is_some()
    }

    /// Closes everything at once rather than one at a time — pressing
    /// `PICKUP_KEY` again while *anything* is open should give a single
    /// quick "get me out of this" escape (e.g. mid-fight), not require
    /// remembering which panel is currently frontmost.
    fn close_all(&mut self) {
        self.dialog = None;
        self.forging_open = false;
        self.vendor = None;
    }
}

/// Which inventory-item index (if any) is awaiting the sell confirmation
/// click — selling is destructive (a socketed item's runes are gone for
/// good, see `DECISIONS.md`'s M7 part 2 planning entry) and the user
/// explicitly asked for a confirm step, unlike buying. Reset whenever the
/// vendor panel closes, so a stale pending confirmation can't survive a
/// reopen against a different vendor's item list.
#[derive(Resource, Default)]
struct PendingSell(Option<usize>);

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
        .add_plugins(EguiPlugin::default())
        .init_resource::<DeltaSeconds>()
        .init_resource::<LocalPlayer>()
        .init_resource::<InventoryOpen>()
        .init_resource::<CharacterPanelOpen>()
        .init_resource::<InteractionPanels>()
        .init_resource::<PendingSell>()
        .add_systems(Startup, (setup_scene, connect_to_server))
        .add_systems(
            Update,
            (
                update_delta_seconds,
                init_replicated_players,
                init_replicated_enemies,
                init_replicated_item_drops,
                init_replicated_interactables,
                player_input_system,
                interaction_trigger_system,
                sync_transform_system,
                party_camera_system,
                player_appearance_system,
                facing_indicator_system,
            )
                .chain(),
        )
        // egui's widget interaction (hover/click) only works for `.show()`
        // calls made from within this schedule — it runs inside egui's own
        // begin/end-pass wrapper (`PostUpdate`'s `EguiPostUpdateSet::EndPass`,
        // after our own `Update` chain above has already run, so replicated
        // state like `LocalPlayer` is current). Panels drawn from plain
        // `Update` still render (one pass behind), which is why this bug
        // read as "visible but unclickable" rather than a crash or a blank
        // panel — found via a live playtest report, not caught by any
        // automated check, since nothing exercises real mouse clicks.
        .add_systems(
            EguiPrimaryContextPass,
            (
                hud_system,
                party_status_system,
                minimap_system,
                inventory_panel_system,
                character_panel_system,
                dialog_panel_system,
                forging_panel_system,
                vendor_panel_system,
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

    let interactable_templates =
        load_all_interactable_templates(Path::new(INTERACTABLE_TEMPLATES_DIR))
            .unwrap_or_else(|error| panic!("failed to load interactable templates: {error}"));
    commands.insert_resource(InteractableTemplates(interactable_templates));

    let rune_templates = load_all_rune_templates(Path::new(RUNE_TEMPLATES_DIR))
        .unwrap_or_else(|error| panic!("failed to load rune templates: {error}"));
    commands.insert_resource(RuneTemplates(rune_templates));

    let item_templates = load_all_item_templates(Path::new(ITEM_TEMPLATES_DIR))
        .unwrap_or_else(|error| panic!("failed to load item templates: {error}"));
    commands.insert_resource(ItemTemplates(item_templates));

    let vendor_templates = load_all_vendor_templates(Path::new(VENDOR_TEMPLATES_DIR))
        .unwrap_or_else(|error| panic!("failed to load vendor templates: {error}"));
    commands.insert_resource(VendorTemplates(vendor_templates));
}

/// Loads this client's persistent character ID from the given path
/// (`CHARACTER_ID_PATH`, or the first CLI arg if given — see its doc
/// comment), or generates and saves a new random one if the file is
/// missing or unreadable as a `u128`.
fn load_or_create_character_id(path: &str) -> u128 {
    if let Ok(contents) = fs::read_to_string(path) {
        if let Ok(id) = contents.trim().parse() {
            return id;
        }
    }
    let id: u128 = rand::random();
    fs::write(path, id.to_string())
        .unwrap_or_else(|error| panic!("failed to save character id to {path}: {error}"));
    id
}

/// Blocking terminal prompt — the closest non-UI stand-in for "enter a
/// password" until M8 builds a real menu (see DECISIONS.md's
/// identity-model entry); the game/character password isn't remembered
/// client-side, so this runs every connection attempt.
fn prompt(label: &str) -> String {
    print!("{label}: ");
    let _ = io::stdout().flush();
    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);
    input.trim().to_string()
}

fn connect_to_server(mut commands: Commands, channels: Res<RepliconChannels>) -> Result<()> {
    let server_channels_config = channels.server_configs();
    let client_channels_config = channels.client_configs();

    let client = RenetClient::new(ConnectionConfig {
        server_channels_config,
        client_channels_config,
        ..Default::default()
    });

    let character_id_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| CHARACTER_ID_PATH.to_string());
    let character_id = load_or_create_character_id(&character_id_path);
    println!("Character ID: {character_id}");
    let auth = ConnectAuth {
        game_password: prompt("Game password"),
        character_id,
        character_password: prompt("Character password"),
    };
    let user_data = auth.encode()?;

    let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
    let client_id = current_time.as_millis() as u64;
    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), SERVER_PORT);
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    let authentication = ClientAuthentication::Unsecure {
        client_id,
        protocol_id: PROTOCOL_ID,
        server_addr,
        user_data: Some(user_data),
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

/// Reacts to newly-replicated `ItemDrop`s (spawned server-side by
/// `combat::death_system`'s loot roll — see `server`'s
/// `tag_item_drops_for_replication`). Purely a placeholder visual (a small
/// colored square distinguishing item vs. rune drops) until real item/rune
/// sprites exist; there's no inventory/equipment UI yet either (M8), so
/// this is just enough to make pickup/equip/socket testable live.
fn init_replicated_item_drops(
    mut commands: Commands,
    new_drops: Query<(Entity, &ItemDrop), Added<ItemDrop>>,
) {
    for (entity, drop) in &new_drops {
        let color = match drop.0 {
            DroppedLoot::Item(_) => ITEM_DROP_COLOR,
            DroppedLoot::Rune(_) => RUNE_DROP_COLOR,
            DroppedLoot::Currency(_) => CURRENCY_DROP_COLOR,
        };
        commands.entity(entity).insert((
            Sprite::from_color(color, Vec2::splat(DROP_SPRITE_SIZE)),
            Transform::default(),
        ));
    }
}

/// Reacts to newly-replicated `Interactable`s (spawned server-side from the
/// map's "interactables" object layer — see `server`'s
/// `spawn_interactables`). Purely a placeholder visual until M8 step 9's
/// dialog panel and step 10's forging UI give these a reason to look
/// distinct per template — same "render, don't simulate" split as
/// `init_replicated_enemies`.
fn init_replicated_interactables(
    mut commands: Commands,
    new_interactables: Query<Entity, Added<Interactable>>,
) {
    for entity in &new_interactables {
        commands.entity(entity).insert((
            Sprite::from_color(INTERACTABLE_COLOR, Vec2::splat(INTERACTABLE_SPRITE_SIZE)),
            Transform::default(),
        ));
    }
}

fn update_delta_seconds(time: Res<Time>, mut delta: ResMut<DeltaSeconds>) {
    delta.0 = time.delta_secs();
}

#[allow(clippy::too_many_arguments)] // one MessageWriter per input type, inherent to this system's job
fn player_input_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    egui_wants_input: Res<EguiWantsInput>,
    local_player: Res<LocalPlayer>,
    mut inventory_open: ResMut<InventoryOpen>,
    mut move_input: MessageWriter<MoveInput>,
    mut attack_input: MessageWriter<AttackInput>,
    mut revive_input: MessageWriter<ReviveInput>,
    mut cast_skill_input: MessageWriter<CastSkillInput>,
    mut pickup_input: MessageWriter<PickupItemInput>,
    mut character_panel_open: ResMut<CharacterPanelOpen>,
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

    for (key, skill_id) in SKILL_HOTKEYS {
        if keyboard.just_pressed(key) {
            cast_skill_input.write(CastSkillInput {
                skill_id: skill_id.to_string(),
            });
        }
    }

    revive_input.write(ReviveInput {
        held: keyboard.pressed(KeyCode::KeyF),
    });

    // Input-focus guard (M8 step 4): WASD/Space/F(revive)/1-3 above are
    // never gated, per CLAUDE.md — a menu is an overlay, not a pause, and
    // combat/movement must keep working while one's open. Everything
    // below is a non-protected hotkey, gated on real egui keyboard focus
    // (a `TextEdit`-style widget, not just the mouse hovering a panel —
    // hover alone would make `INVENTORY_TOGGLE_KEY` unable to close the
    // very panel the cursor happens to be resting on) so a panel click
    // can't also register as one of these actions. This panel has no
    // text field yet, so `wants_keyboard_input()` is always false today —
    // the guard is real, correctly-wired infrastructure with no
    // observable effect until a future panel adds one.
    if egui_wants_input.wants_keyboard_input() {
        return;
    }

    if keyboard.just_pressed(INVENTORY_TOGGLE_KEY) {
        inventory_open.0 = !inventory_open.0;
    }

    if keyboard.just_pressed(CHARACTER_TOGGLE_KEY) {
        character_panel_open.0 = !character_panel_open.0;
    }

    if keyboard.just_pressed(PICKUP_KEY) {
        pickup_input.write(PickupItemInput);
    }
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

/// Same party membership as `PartyPositions`, extended with the fields the
/// M8.5 party-status panel/minimap need — kept as a separate type rather
/// than widening `PartyPositions` itself, since `party_centroid_and_spread`
/// below assumes that query yields a bare `&Position` per match.
type PartyStatus<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Position,
        &'static Health,
        Option<&'static Od>,
        Has<Downed>,
        Has<Stunned>,
    ),
    Or<(With<Player>, With<RemotePlayer>)>,
>;

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
    (Entity, &'static mut Sprite, Has<Downed>, Has<Stunned>),
    Or<(With<Player>, With<RemotePlayer>)>,
>;

/// Fully recomputes every party member's sprite color each frame, rather
/// than overlaying a tint on top of whatever color was there before — that
/// overlay approach can't un-tint a `RemotePlayer` once `Downed` is
/// removed (ally-revive), since nothing else runs every frame to restore
/// their base color. `Downed` wins over `Stunned`, which wins over the
/// leash-warning tint; only the local player gets a leash-warning tint at
/// all (the boundary itself is enforced server-side, see DESIGN.md's
/// Camera & movement section — this is cosmetic feedback, not the real HUD
/// indicator planned for M8). Enemies don't get a `Stunned` tint yet — see
/// DECISIONS.md for why that's deferred, not an oversight.
fn player_appearance_system(
    local_player: Res<LocalPlayer>,
    party: PartyPositions,
    mut sprites: PartySprites,
) {
    let centroid = party_centroid_and_spread(&party).map(|(centroid, _)| centroid);
    let warning_threshold = (LEASH_DISTANCE / 2.0) * LEASH_WARNING_RATIO;

    for (entity, mut sprite, downed, stunned) in &mut sprites {
        sprite.color = if downed {
            DOWNED_COLOR
        } else if stunned {
            STUNNED_COLOR
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

/// Draws a facing-direction arrow for every entity with a `Facing` —
/// players and enemies alike, local or remote, with no special-casing.
/// Gizmos are immediate-mode (redrawn from scratch every frame from
/// whatever `Position`/`Facing` currently hold), so there's no spawned
/// indicator entity or lifecycle to manage, and no stale-state risk like
/// the sprite-tint overlay bug fixed in `player_appearance_system`.
/// Placeholder visualization, not real art — see MECHANICS.md's Combat
/// section for what `Facing` means.
fn facing_indicator_system(query: Query<(&Position, &Facing)>, mut gizmos: Gizmos) {
    for (position, facing) in &query {
        let start = Vec2::new(position.x, position.y);
        let end = start + Vec2::new(facing.x, facing.y) * FACING_ARROW_LENGTH;
        gizmos.arrow_2d(start, end, FACING_ARROW_COLOR);
    }
}

/// `Od`/`SkillCooldowns` are `Option` for the same reason as elsewhere —
/// only players have either, and `SkillCooldowns` only gains an entry once
/// a skill's actually been cast (see `game_core::skill`).
type LocalPlayerHud<'w, 's> = Query<
    'w,
    's,
    (
        &'static Health,
        Option<&'static Od>,
        Option<&'static SkillCooldowns>,
        &'static Currency,
        Has<Downed>,
    ),
>;

/// Read-only egui HUD: health/od bars, skill cooldowns, currency balance,
/// and a downed-state indicator. Skill rows use the existing fixed `1`-`3`
/// hotkeys from M6 (`SKILL_HOTKEYS`) rather than `KnownSkills` — the
/// skill-tree UI that would ever populate `KnownSkills` with something
/// other than "empty" doesn't exist yet (see ROADMAP.md's M8 step 6).
/// Nothing here is clickable, so it doesn't need the input-focus guard
/// `player_input_system` applies for `inventory_panel_system` below.
fn hud_system(mut contexts: EguiContexts, local_player: Res<LocalPlayer>, query: LocalPlayerHud) {
    let Some(entity) = local_player.0 else {
        return;
    };
    let Ok((health, od, skill_cooldowns, currency, downed)) = query.get(entity) else {
        return;
    };
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    egui::Window::new("hud")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(10.0, 10.0))
        .show(ctx, |ui| {
            ui.add(
                egui::ProgressBar::new(health.current / health.max)
                    .text(format!("HP {:.0}/{:.0}", health.current, health.max)),
            );
            if let Some(od) = od {
                ui.add(
                    egui::ProgressBar::new(od.current / od.max)
                        .text(format!("Od {:.0}/{:.0}", od.current, od.max)),
                );
            }
            ui.label(format!("Coins: {}", currency.0));
            if downed {
                ui.colored_label(egui::Color32::RED, "DOWNED");
            }

            ui.separator();
            for (index, (_, skill_id)) in SKILL_HOTKEYS.iter().enumerate() {
                let remaining = skill_cooldowns
                    .and_then(|cooldowns| cooldowns.0.get(*skill_id))
                    .copied()
                    .unwrap_or(0.0);
                let label = if remaining > 0.0 {
                    format!("{}: {skill_id} ({remaining:.1}s)", index + 1)
                } else {
                    format!("{}: {skill_id} ready", index + 1)
                };
                ui.label(label);
            }
        });
}

/// Lists every party member's (local + remote) HP/Od bars and downed/
/// stunned status — the M8 gap `ROADMAP.md` flagged as unblocked but
/// unbuilt (M8.5). All fields are already replicated
/// (`Health`/`Od`/`Downed`/`Stunned`), so this needs no new protocol work,
/// just a wider query than any single-panel system used before. Nothing
/// here is clickable, so (like `hud_system`) it doesn't need the
/// input-focus guard.
fn party_status_system(
    mut contexts: EguiContexts,
    local_player: Res<LocalPlayer>,
    party: PartyStatus,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    egui::Window::new("Party")
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-10.0, 10.0))
        .resizable(false)
        .show(ctx, |ui| {
            for (entity, _, health, od, downed, stunned) in &party {
                let label = if Some(entity) == local_player.0 {
                    "You"
                } else {
                    "Ally"
                };
                ui.horizontal(|ui| {
                    ui.label(label);
                    ui.add(
                        egui::ProgressBar::new(health.current / health.max)
                            .text(format!("HP {:.0}/{:.0}", health.current, health.max)),
                    );
                    if let Some(od) = od {
                        ui.add(
                            egui::ProgressBar::new(od.current / od.max)
                                .text(format!("Od {:.0}/{:.0}", od.current, od.max)),
                        );
                    }
                    if downed {
                        ui.colored_label(egui::Color32::RED, "DOWNED");
                    } else if stunned {
                        ui.colored_label(egui::Color32::YELLOW, "STUNNED");
                    }
                });
            }
        });
}

/// Small fixed-size minimap plotting every party member's position against
/// the map's known fixed extent (`MAP_WORLD_WIDTH`/`MAP_WORLD_HEIGHT`) —
/// placeholder colored dots, not real terrain art; that's the whole point
/// of M8.5, getting lighting/ambience tunable before any real art exists.
fn minimap_system(mut contexts: EguiContexts, local_player: Res<LocalPlayer>, party: PartyStatus) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    egui::Window::new("minimap")
        .title_bar(false)
        .resizable(false)
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(10.0, -10.0))
        .show(ctx, |ui| {
            let (response, painter) =
                ui.allocate_painter(egui::vec2(MINIMAP_SIZE, MINIMAP_SIZE), egui::Sense::hover());
            let rect = response.rect;
            painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(180));

            for (entity, position, _, _, _, _) in &party {
                let normalized_x = (position.x / MAP_WORLD_WIDTH).clamp(0.0, 1.0);
                let normalized_y = (-position.y / MAP_WORLD_HEIGHT).clamp(0.0, 1.0);
                let point =
                    rect.min + egui::vec2(normalized_x * MINIMAP_SIZE, normalized_y * MINIMAP_SIZE);
                let color = if Some(entity) == local_player.0 {
                    MINIMAP_PLAYER_COLOR
                } else {
                    MINIMAP_REMOTE_COLOR
                };
                painter.circle_filled(point, 3.0, color);
            }
        });
}

/// One row per socket: "filled" (by rune id) or "empty" — sockets aren't
/// named individually anywhere else in the UI, so this is purely a
/// display convenience for the panel below.
fn socket_summary(item: &Item) -> String {
    let filled = item.sockets.iter().filter(|s| s.is_some()).count();
    format!(
        "{} ({filled}/{} sockets)",
        item.template_key,
        item.sockets.len()
    )
}

/// Click-driven inventory + equip/unequip panel — replaces M7's `F1`-`F6`
/// hotkey stand-ins (see `INVENTORY_TOGGLE_KEY`'s doc comment). Toggled by
/// `INVENTORY_TOGGLE_KEY`, and by egui's own window close button via the
/// shared `InventoryOpen` flag passed to `egui::Window::open`. Displays
/// items by their raw `template_key` (e.g. `"rusty_sword"`) — `ItemTemplate`
/// has no separate display-name field yet, same placeholder-text treatment
/// as the HUD's skill rows.
fn inventory_panel_system(
    mut contexts: EguiContexts,
    local_player: Res<LocalPlayer>,
    mut inventory_open: ResMut<InventoryOpen>,
    query: Query<(&Inventory, &Equipment)>,
    mut equip_input: MessageWriter<EquipItemInput>,
    mut unequip_input: MessageWriter<UnequipItemInput>,
) {
    if !inventory_open.0 {
        return;
    }
    let Some(entity) = local_player.0 else {
        return;
    };
    let Ok((inventory, equipment)) = query.get(entity) else {
        return;
    };
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let mut equip_clicked = None;
    let mut unequip_clicked = None;

    egui::Window::new("Inventory")
        .open(&mut inventory_open.0)
        .show(ctx, |ui| {
            ui.heading("Equipped");
            for (label, slot, item) in [
                ("Weapon", EquipSlot::Weapon, &equipment.weapon),
                ("Armor", EquipSlot::Armor, &equipment.armor),
                ("Helmet", EquipSlot::Helmet, &equipment.helmet),
            ] {
                ui.horizontal(|ui| match item {
                    Some(item) => {
                        ui.label(format!("{label}: {}", socket_summary(item)));
                        if ui.button("Unequip").clicked() {
                            unequip_clicked = Some(slot);
                        }
                    }
                    None => {
                        ui.label(format!("{label}: (empty)"));
                    }
                });
            }

            ui.separator();
            ui.heading("Inventory");
            if inventory.0.is_empty() {
                ui.label("(empty)");
            }
            for (index, item) in inventory.0.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(socket_summary(item));
                    if ui.button("Equip").clicked() {
                        equip_clicked = Some(index);
                    }
                });
            }
        });

    // Deferred until after `show` closes: the closure above borrows `ui`
    // (and transitively `ctx`) mutably, and `MessageWriter::write` doesn't
    // need to happen inside it.
    if let Some(inventory_index) = equip_clicked {
        equip_input.write(EquipItemInput { inventory_index });
    }
    if let Some(slot) = unequip_clicked {
        unequip_input.write(UnequipItemInput { slot });
    }
}

/// Label + the `Stat` variant `AllocateStatPointInput` carries, for each
/// stat the character panel lets a player allocate into. `bonus_max_health`
/// isn't listed: `Stat` has no matching variant (see its doc comment in
/// `game_core::status_effect`), so there's no way to construct a message
/// for it in the first place.
const ALLOCATABLE_STATS: [(&str, Stat); 3] = [
    ("Move Speed", Stat::MoveSpeed),
    ("Crit Chance", Stat::CritChance),
    ("Crit Multiplier", Stat::CritMultiplier),
];

fn stat_bonus(stats: &Stats, stat: Stat) -> f32 {
    match stat {
        Stat::MoveSpeed => stats.bonus_move_speed,
        Stat::CritChance => stats.bonus_crit_chance,
        Stat::CritMultiplier => stats.bonus_crit_multiplier,
    }
}

type LocalPlayerCharacter<'w, 's> = Query<
    'w,
    's,
    (
        &'static Level,
        &'static UnspentStatPoints,
        &'static Stats,
        &'static UnspentSkillPoints,
        &'static KnownSkills,
    ),
>;

/// Click-driven level-up panel: stat-point allocation (flat list, `+1` per
/// click via `AllocateStatPointInput`) and skill learning (`Learn`/`+1` via
/// `LearnSkillInput`) — the two gaps `UnspentStatPoints`/`UnspentSkillPoints`
/// have had since M5/M6 (M8 step 2 already wired the stat bonuses into
/// combat/movement, so this panel is meaningful from the moment it exists).
/// Skill rows reuse `SKILL_HOTKEYS`' fixed id list rather than a client-side
/// `SkillLibrary` (which doesn't exist) — same placeholder-content pattern
/// as the HUD's cooldown rows. Flat spend-a-point list, no prerequisite
/// tree topology: nothing in `MECHANICS.md`/`DESIGN.md` specifies an actual
/// tree shape.
fn character_panel_system(
    mut contexts: EguiContexts,
    local_player: Res<LocalPlayer>,
    mut panel_open: ResMut<CharacterPanelOpen>,
    query: LocalPlayerCharacter,
    mut allocate_input: MessageWriter<AllocateStatPointInput>,
    mut learn_input: MessageWriter<LearnSkillInput>,
) {
    if !panel_open.0 {
        return;
    }
    let Some(entity) = local_player.0 else {
        return;
    };
    let Ok((level, stat_points, stats, skill_points, known_skills)) = query.get(entity) else {
        return;
    };
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let mut allocate_clicked = None;
    let mut learn_clicked = None;

    egui::Window::new("Character")
        .open(&mut panel_open.0)
        .show(ctx, |ui| {
            ui.heading(format!(
                "Level {} (XP {:.0}/{:.0})",
                level.level,
                level.xp,
                xp_required(level.level)
            ));

            ui.separator();
            ui.heading(format!("Stat points: {}", stat_points.0));
            for (label, stat) in ALLOCATABLE_STATS {
                ui.horizontal(|ui| {
                    ui.label(format!("{label}: +{:.2}", stat_bonus(stats, stat)));
                    if ui
                        .add_enabled(stat_points.0 > 0, egui::Button::new("+1"))
                        .clicked()
                    {
                        allocate_clicked = Some(stat);
                    }
                });
            }

            ui.separator();
            ui.heading(format!("Skill points: {}", skill_points.0));
            for (_, skill_id) in SKILL_HOTKEYS {
                let skill_level = known_skills.0.get(skill_id).copied().unwrap_or(0);
                ui.horizontal(|ui| {
                    ui.label(format!("{skill_id}: level {skill_level}"));
                    let button_label = if skill_level == 0 { "Learn" } else { "+1" };
                    if ui
                        .add_enabled(skill_points.0 > 0, egui::Button::new(button_label))
                        .clicked()
                    {
                        learn_clicked = Some(skill_id);
                    }
                });
            }
        });

    // Deferred until after `show` closes — same reason as
    // `inventory_panel_system`'s deferred writes.
    if let Some(stat) = allocate_clicked {
        allocate_input.write(AllocateStatPointInput { stat });
    }
    if let Some(skill_id) = learn_clicked {
        learn_input.write(LearnSkillInput {
            skill_id: skill_id.to_string(),
        });
    }
}

/// Triggers whichever interaction panel(s) apply: on `PICKUP_KEY`, finds
/// the nearest in-range `Interactable` via
/// `game_core::nearest_interactable_in_range` — the same priority rule
/// `server::interact_or_pickup_system` uses for the real effect grant,
/// shared rather than duplicated so the two can never disagree on which
/// `Interactable` is "the" nearest one. Opens every panel its template's
/// `opens_panels` declares (a blacksmith opens both forging and vendor at
/// once); if it declares none, falls back to `dialog` text instead — same
/// either-or priority M8 originally had for a single panel. Both are
/// purely local reads of already-replicated data, no round-trip (see
/// `DECISIONS.md`'s M8 planning entry) — the forging/vendor panels'
/// actual socket/buy/sell actions are still a real round-trip, gated
/// server-side by `game_core::nearest_interactable_with_panel`. Gated
/// behind the same input-focus guard as every other non-protected hotkey
/// (see `player_input_system`).
///
/// `PICKUP_KEY` toggles: if anything is already open, the next press just
/// closes all of it (`InteractionPanels::close_all`) rather than
/// re-checking proximity — a quick one-key dismiss for when it pops up
/// mid-fight (e.g. picking up loot in range of a dialog `Interactable`),
/// not just a window's own mouse-driven close button. The underlying
/// interact/pickup request still fires every press regardless (see
/// `player_input_system`) — this only toggles the local panel(s), not the
/// server-resolved action.
fn interaction_trigger_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    egui_wants_input: Res<EguiWantsInput>,
    local_player: Res<LocalPlayer>,
    positions: Query<&Position>,
    interactables: Query<(&Position, &Interactable)>,
    templates: Res<InteractableTemplates>,
    mut panels: ResMut<InteractionPanels>,
) {
    if egui_wants_input.wants_keyboard_input() {
        return;
    }
    if !keyboard.just_pressed(PICKUP_KEY) {
        return;
    }
    if panels.any_open() {
        panels.close_all();
        return;
    }
    let Some(entity) = local_player.0 else {
        return;
    };
    let Ok(actor_pos) = positions.get(entity) else {
        return;
    };
    let Some(interactable) = nearest_interactable_in_range(actor_pos, interactables.iter()) else {
        return;
    };
    let Some((_, template)) = templates
        .0
        .iter()
        .find(|(key, _)| *key == interactable.template_key)
    else {
        return;
    };

    let mut opened_any = false;
    if template.opens_panels.iter().any(|p| p == FORGING_PANEL_ID) {
        panels.forging_open = true;
        opened_any = true;
    }
    if template.opens_panels.iter().any(|p| p == VENDOR_PANEL_ID) {
        panels.vendor = Some(interactable.template_key.clone());
        opened_any = true;
    }
    if !opened_any {
        if let Some(dialog) = &template.dialog {
            panels.dialog = Some(dialog.clone());
        }
    }
}

/// Read-only dialog window shown when `InteractionPanels::dialog` is set;
/// closed via egui's own close button, which clears it back to `None`. No
/// text input, so (like the HUD) doesn't need the input-focus guard.
fn dialog_panel_system(mut contexts: EguiContexts, mut panels: ResMut<InteractionPanels>) {
    let Some(text) = panels.dialog.clone() else {
        return;
    };
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let mut open = true;
    egui::Window::new("Dialog").open(&mut open).show(ctx, |ui| {
        ui.label(text);
    });
    if !open {
        panels.dialog = None;
    }
}

/// One row per socket: filled sockets get an "Unsocket" button (still
/// free — see `game_core::unsocket_rune`); empty ones get one "Socket
/// <rune_id> (xN) - <cost>g" button per rune the player actually has in
/// stock (`RuneInventory`), disabled if `currency` can't cover that
/// rune's `socket_cost` (looked up from `RuneTemplates`, purely for
/// display/gating — the server independently enforces the real charge in
/// `game_core::socket_rune`, so this is a convenience, not the source of
/// truth). Reads real rune ids straight off replicated data rather than a
/// hardcoded list, so a new rune template needs no client change to
/// become forgeable. Rune ids are sorted for a stable button order
/// (`RuneInventory`'s `HashMap` iteration order isn't otherwise
/// guaranteed frame-to-frame).
#[allow(clippy::too_many_arguments)] // one input per rendered/interactive concern, inherent to this row's job
fn socket_row(
    ui: &mut egui::Ui,
    slot: EquipSlot,
    socket_index: usize,
    socket: &Option<String>,
    runes: &RuneInventory,
    rune_templates: &RuneTemplates,
    currency: &Currency,
    socket_clicked: &mut Option<(EquipSlot, usize, String)>,
    unsocket_clicked: &mut Option<(EquipSlot, usize)>,
) {
    ui.horizontal(|ui| match socket {
        Some(rune_id) => {
            ui.label(format!("Socket {socket_index}: {rune_id}"));
            if ui.button("Unsocket").clicked() {
                *unsocket_clicked = Some((slot, socket_index));
            }
        }
        None => {
            ui.label(format!("Socket {socket_index}: (empty)"));
            let mut rune_ids: Vec<_> = runes.0.iter().filter(|(_, count)| **count > 0).collect();
            rune_ids.sort_by_key(|(rune_id, _)| rune_id.as_str());
            for (rune_id, count) in rune_ids {
                let cost = rune_templates
                    .0
                    .iter()
                    .find(|(key, _)| key == rune_id)
                    .map(|(_, template)| template.socket_cost)
                    .unwrap_or(0);
                let affordable = currency.0 >= cost;
                let label = format!("{rune_id} (x{count}) - {cost}g");
                if ui
                    .add_enabled(affordable, egui::Button::new(label))
                    .clicked()
                {
                    *socket_clicked = Some((slot, socket_index, rune_id.clone()));
                }
            }
        }
    });
}

/// Forging panel: shown when `InteractionPanels::forging_open` is set
/// (opened by `interaction_trigger_system` near a blacksmith-kind
/// `Interactable`). Lists each equipped item's sockets with socket/unsocket
/// buttons — replaces M7's `F7`/`F8`/`F9` hotkey stand-ins (removed now
/// that the panel exists, per `ROADMAP.md`, same precedent as the
/// inventory panel replacing `F1`-`F6`). The actual socket/unsocket is
/// still a server round-trip, proximity-gated there too
/// (`game_core::nearest_interactable_with_panel`) — this panel can be
/// left open while walking away, at which point those requests just
/// silently no-op, same "invalid action, no client-side feedback yet"
/// treatment every other action system already has.
#[allow(clippy::too_many_arguments)] // one query/resource per rendered/interactive concern, inherent to this panel's job
fn forging_panel_system(
    mut contexts: EguiContexts,
    local_player: Res<LocalPlayer>,
    mut panels: ResMut<InteractionPanels>,
    query: Query<(&Equipment, &RuneInventory, &Currency)>,
    rune_templates: Res<RuneTemplates>,
    mut socket_input: MessageWriter<SocketRuneInput>,
    mut unsocket_input: MessageWriter<UnsocketRuneInput>,
) {
    if !panels.forging_open {
        return;
    }
    let Some(entity) = local_player.0 else {
        return;
    };
    let Ok((equipment, runes, currency)) = query.get(entity) else {
        return;
    };
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let mut open = true;
    let mut socket_clicked = None;
    let mut unsocket_clicked = None;

    egui::Window::new("Forging")
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(format!("Coins: {}", currency.0));
            ui.separator();
            for (label, slot, item) in [
                ("Weapon", EquipSlot::Weapon, &equipment.weapon),
                ("Armor", EquipSlot::Armor, &equipment.armor),
                ("Helmet", EquipSlot::Helmet, &equipment.helmet),
            ] {
                ui.heading(label);
                match item {
                    Some(item) => {
                        for (socket_index, socket) in item.sockets.iter().enumerate() {
                            socket_row(
                                ui,
                                slot,
                                socket_index,
                                socket,
                                runes,
                                &rune_templates,
                                currency,
                                &mut socket_clicked,
                                &mut unsocket_clicked,
                            );
                        }
                    }
                    None => {
                        ui.label("(nothing equipped)");
                    }
                }
                ui.separator();
            }
        });

    // Deferred until after `show` closes — same reason as
    // `inventory_panel_system`'s deferred writes.
    if let Some((slot, socket_index, rune_id)) = socket_clicked {
        socket_input.write(SocketRuneInput {
            slot,
            socket_index,
            rune_id,
        });
    }
    if let Some((slot, socket_index)) = unsocket_clicked {
        unsocket_input.write(UnsocketRuneInput { slot, socket_index });
    }
    if !open {
        panels.forging_open = false;
    }
}

/// Vendor panel: shown when `InteractionPanels::vendor` holds a vendor's
/// `template_key` (opened by `interaction_trigger_system` near a
/// vendor-kind `Interactable` — a merchant, or a blacksmith that also
/// vends). Buy is one click; sell requires an explicit confirm step first
/// (`PendingSell`) since it's destructive — a socketed item's runes are
/// gone for good on sell, per `DECISIONS.md`'s M7 part 2 planning entry.
/// The sell-price preview reuses `game_core::socketed_item_sell_value`
/// against this client's own locally-loaded `ItemTemplates`/
/// `RuneTemplates`, the same formula the server's authoritative
/// `sell_item` uses, so the two can't disagree about what an item is
/// actually worth. Buying/selling are still real round-trips, proximity-
/// gated server-side (`game_core::nearest_interactable_with_panel`) —
/// same "can be left open while walking away, requests just silently
/// no-op" treatment as the forging panel.
#[allow(clippy::too_many_arguments)] // one query/resource per rendered/interactive concern, inherent to this panel's job
fn vendor_panel_system(
    mut contexts: EguiContexts,
    local_player: Res<LocalPlayer>,
    mut panels: ResMut<InteractionPanels>,
    mut pending_sell: ResMut<PendingSell>,
    query: Query<(&Inventory, &Currency)>,
    vendor_templates: Res<VendorTemplates>,
    item_templates: Res<ItemTemplates>,
    rune_templates: Res<RuneTemplates>,
    mut buy_input: MessageWriter<BuyItemInput>,
    mut sell_input: MessageWriter<SellItemInput>,
) {
    let Some(vendor_key) = panels.vendor.clone() else {
        return;
    };
    let Some(entity) = local_player.0 else {
        return;
    };
    let Ok((inventory, currency)) = query.get(entity) else {
        return;
    };
    let Some((_, vendor_template)) = vendor_templates
        .0
        .iter()
        .find(|(key, _)| *key == vendor_key)
    else {
        return;
    };
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let mut open = true;
    let mut buy_clicked = None;
    let mut sell_confirmed = None;

    egui::Window::new("Vendor").open(&mut open).show(ctx, |ui| {
        ui.label(format!("Coins: {}", currency.0));
        ui.separator();

        ui.heading("Buy");
        for (index, listing) in vendor_template.inventory.iter().enumerate() {
            let affordable = currency.0 >= listing.price;
            let label = format!("{} - {}g", listing.item_template_key, listing.price);
            if ui
                .add_enabled(affordable, egui::Button::new(label))
                .clicked()
            {
                buy_clicked = Some(index);
            }
        }

        ui.separator();
        ui.heading("Sell");
        for (index, item) in inventory.0.iter().enumerate() {
            let base_sell_value = item_templates
                .0
                .iter()
                .find(|(key, _)| *key == item.template_key)
                .map(|(_, template)| template.sell_value)
                .unwrap_or(0);
            let sell_value = socketed_item_sell_value(base_sell_value, &item.sockets, |rune_id| {
                rune_templates
                    .0
                    .iter()
                    .find(|(key, _)| key == rune_id)
                    .map(|(_, template)| template.socket_cost)
            });

            ui.horizontal(|ui| {
                ui.label(format!("{} ({sell_value}g)", item.template_key));
                if pending_sell.0 == Some(index) {
                    if ui.button("Confirm").clicked() {
                        sell_confirmed = Some(index);
                    }
                    if ui.button("Cancel").clicked() {
                        pending_sell.0 = None;
                    }
                } else if ui.button("Sell").clicked() {
                    pending_sell.0 = Some(index);
                }
            });
        }
    });

    if let Some(listing_index) = buy_clicked {
        buy_input.write(BuyItemInput { listing_index });
    }
    if let Some(inventory_index) = sell_confirmed {
        sell_input.write(SellItemInput { inventory_index });
        pending_sell.0 = None;
    }
    if !open {
        panels.vendor = None;
        pending_sell.0 = None;
    }
}
