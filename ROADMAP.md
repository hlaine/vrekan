# ROADMAP.md

Milestone sequence for v1. Living document — check items off, adjust order if
reality demands it, don't treat this as a rigid contract. Each milestone should
be small enough to review in a sitting; break a milestone into individual
Claude Code tasks as you start it, not all upfront.

## M0 — Workspace scaffold
- [x] Cargo workspace with all crates from `CLAUDE.md`, compiling but empty
- [x] CI (fmt check, clippy deny-warnings, test) passing on a trivial commit
- [x] `LICENSE`, `.gitignore`

## M1 — Single-player core loop (no networking yet)
- [x] Player entity: movement (WASD), basic melee attack
- [x] One enemy type, hardcoded, with health and a simple AI pattern
- [x] `apply_damage` and death handling in `game_core`, unit tested
- [x] Runs and is playable locally, single client, no server

## M2 — Data-driven content
- [x] RON schema + loader for enemy templates in `content`
- [x] Convert the M1 hardcoded enemy into a data-driven template
- [x] Add one more enemy type purely via a new `.ron` file, no engine changes
  — this is the test that the extensibility goal actually works

## M3 — Basic networking
- [x] Headless `server` binary, `MinimalPlugins`
- [x] bevy_replicon wired up: position replication for a moving entity
- [x] Two clients connect to one local server, see each other move

## M3.5 — Shared camera, leash, and map collision
- [x] Camera driven by party centroid (both clients), zoom scales with party
  spread — see `DESIGN.md`'s Camera & movement section
- [x] Hard leash: players can't exceed the camera's max spread, plus a
  lightweight visual indicator at the limit (not the real HUD, that's M8)
- [x] Map collision: freeform polygon colliders authored in Tiled, a first
  test map (`assets/maps/valley.tmx`) with a valley-style narrow pass
  between blocking mountain shapes. `avian2d` is the physics backend
  (server-only, `RigidBody::Dynamic` players against `RigidBody::Static`
  colliders — `Kinematic` bodies aren't affected by collisions in avian2d,
  so this isn't the naive choice it looks like). Client renders the map via
  `bevy_ecs_tiled`; the server parses the same file with the plain `tiled`
  crate instead, since `bevy_ecs_tiled` hard-depends on `bevy_render`
  regardless of features (confirmed empirically — not just a Cargo-feature
  toggle away) — see `CLAUDE.md`'s feature-unification note and
  `server/src/main.rs`'s `spawn_map_colliders` doc comment for the anchor
  convention keeping the two in sync. Both `tiled` and `avian2d` are new
  external dependencies per `CLAUDE.md`'s review rule — flagged and
  confirmed.
- [x] First real camera system — M1 only ever spawned a static `Camera2d` at
  the origin, so this isn't a replacement of prior follow-camera behavior

## M4 — Combat & damage type system, networked
- [x] `DamageType` + resistance system from `game_core`, server-authoritative
  — see `MECHANICS.md` for the damage/crit formula shape. `DamageType` is a
  data-keyed string (not a fixed enum), `Resistances`/`CombatStats` are new
  components, `resolve_damage` takes a generic `rng: &mut impl Rng` for
  deterministic tests (new `rand` dependency in `game_core`, flagged and
  confirmed). Enemy templates gained `melee_damage_type`/`crit_chance`/
  `crit_multiplier`/`resistances` fields. Confirmed live: a player killed an
  enemy with a resistance value in a real playtest, and (accidentally, while
  debugging) confirmed the reverse direction too — see `DECISIONS.md` for
  two real bugs this surfaced: `AttackInput`'s reliable channel silently
  stopped delivering after ~8 messages, and player death currently
  disconnects the client (no downed state yet, tracked below).
- [x] Combat (melee attack from M1) works correctly across client/server —
  enemies now spawn/simulate server-side (`content::spawn_enemy`, replicated
  via `Enemy`/`EnemyKind`/`Health`/`Position`) and attacks are a networked
  `AttackInput` client message resolved authoritatively by
  `game_core::combat::attack_system`; the client only renders replicated
  state. Enemy death (despawn on zero health) confirmed working end-to-end
  in a live two-client playtest — see `DECISIONS.md` if this needs
  revisiting. Caveat found later, during ally-revive testing: that
  confirmation was for player-initiated attacks only. Enemy-initiated
  attacks (`game_core::enemy::ai_system`) didn't actually work with two
  clients connected until the fix described in `DECISIONS.md` — see that
  entry before treating "combat works across client/server" as fully
  verified for anything AI-driven.
- [x] Generic status-effect system — `game_core::status_effect`:
  `EffectKind` (`DamageOverTime`/`Stun`/`StatModifier`) is a small fixed set
  of engine-known behavior shapes, mirroring `DamageType`'s data-vs-engine
  split; an effect's identity/numbers (`EffectDefinition`) is data, same
  principle as enemy/item templates. Stacking (`StackMode`:
  `RefreshDuration`/`AddMagnitude`/`Independent`) is a per-effect field, not
  a global rule, per `MECHANICS.md`. `MeleeAttack` gained `effects:
  Vec<EffectDefinition>`, attachable via content (`content::EffectTemplate`
  RON schema) or server constants, applied to either side of a landed hit
  via `EffectDefinition::applies_to`. Buffs never mutate the base
  `CombatStats`/`MoveSpeed` component — `ActiveEffects::stat_bonus` is
  computed fresh at the point of use, deliberately avoiding the stale-cache
  bug class from ally-revive's original sprite-tint overlay.
  `Stunned` is a replicated marker (mirrors `Downed`) kept in sync by
  `tick_status_effects`, but — unlike `Downed` — a stunned entity stays a
  valid attack target; it can't act, but isn't out of combat. Proven with
  one real content instance per stacking mode: `missionary.ron`'s bleed
  (`AddMagnitude`), `converted_farmer.ron`'s daze/stun (`RefreshDuration`),
  and the player's own "fury" self-buff on landed hits (`Independent`, a
  server constant since player combat stats already are). No enemy-side
  stun visual yet (players only) — see `DECISIONS.md`.
- [x] Player death → "downed" state (not respawn) — see `MECHANICS.md` for
  the downed/revive/wipe rules. `death_system` now branches on a `Player`
  marker: a player at zero health gets a new `Downed` component instead of
  being despawned; everything else (enemies) despawns as before. `Downed`
  entities are excluded from `attack_system`'s target/attacker queries and
  `ai_system`'s player-targeting query — out of combat entirely, matching
  `MECHANICS.md`. Replicated so the client can show a distinct grey tint
  (`client`'s `player_appearance_system`) instead of the downed state
  reading as "input stopped working," the exact confusing symptom this gap
  caused during M4 testing (see `DECISIONS.md`). Full-wipe auto-respawn and
  resurrection-point checkpointing are explicitly M5 scope, not touched
  here — a downed player with no one left to revive them just waits, which
  is correct until M5 lands.
- [x] Ally-revive: a non-downed ally holding **F** within `REVIVE_RANGE`
  (60 units, matching `PLAYER_ATTACK_RANGE`) accumulates `ReviveProgress`
  on the downed entity in `game_core::revive::revive_system`; reaching
  `REVIVE_DURATION_SECS` (3s) restores `REVIVE_HEALTH_FRACTION` (50%) of
  max health and clears `Downed`. Progress isn't banked — walking out of
  range or letting go resets it to zero, a reasonable starting assumption
  per `MECHANICS.md`'s open questions, not a tuned/settled number. New
  `protocol::ReviveInput { held: bool }` client message, sent every frame
  like `MoveInput` (continuous state, `Channel::Unreliable`). No revive
  progress bar yet — that's M8 HUD work; the only client feedback today is
  the downed tint clearing once revived.
- [x] Additions beyond this milestone's original scope, added after a design
  discussion on movement/combat feel:
  - Enemies are solid `avian2d` bodies now (`RigidBody::Dynamic` +
    `Collider::circle` sized from the existing content `size` field,
    mirroring the player's own physics setup in `on_client_connected`) —
    block players, can't be walked through. No explicit `Mass`/
    `ColliderDensity`: avian2d auto-computes mass from collider area ×
    density, so bigger enemies are proportionally heavier and resist being
    shoved for free; current tier-1 enemies (28-30 unit `size`) are ~76-88%
    of the player's mass, so expect mild pushback today, more resistance
    from bigger/later tiers. Enemies also collide with each other and with
    downed players (never removed their collider, so this needed no new
    code) — all via avian2d's default collision layers (everything collides
    with everything unless configured otherwise, verified in source before
    relying on it). Enemies moved off `game_core::movement_system`'s plain
    integrator onto `avian2d` entirely — see `server`'s
    `sync_enemy_velocity_to_physics`; `movement_system` stays in
    `game_core`, tested, just uncalled by this binary now. See `DESIGN.md`'s
    Camera & movement section.
  - Player *and enemy* facing direction (`game_core::Facing`), derived from
    movement input, server-authoritative and replicated — resolves
    `MECHANICS.md`'s open question in favor of giving enemies facing too.
    Groundwork for directional (cone) melee attacks, not yet used to gate
    anything. Visualized client-side with a `bevy_gizmos` arrow
    (`facing_indicator_system`) for both players and enemies — placeholder
    dev visualization, not real art. See `MECHANICS.md`'s Combat section.

## M5 — Progression: leveling & stats
- [x] XP, character level, manual stat point allocation on level up — see
  `MECHANICS.md` for the formula shape. `game_core::progression`:
  `Level { level, xp }` + `xp_required(level)` (quadratic, uncapped),
  `UnspentStatPoints` granted per level via `grant_xp` (handles multiple
  level-ups in one XP grant), `Stats` (bonus_max_health/move_speed/
  crit_chance/crit_multiplier — reuses existing mechanical stats rather
  than inventing new attributes with no hookup yet). `XpReward` on enemies
  (new content field), granted to the killing blow's attacker only in
  `attack_system` — not shared across the party, a starting rule per
  MECHANICS.md's "tune by feel" framing. No stat-*spending* UI yet
  (MECHANICS.md itself defers that to M8) — points accumulate unspent,
  `Stats`' bonuses have no gameplay effect until something can produce a
  nonzero value.
- [x] XP penalty on individual death; full-party-wipe resets in-level
  progress to zero (level itself never drops) — see `MECHANICS.md`.
  `game_core::progression::apply_death_xp_penalty` fires on `Added<Downed>`
  (once per downing, not every tick they stay downed) and reduces `xp` by
  a fixed fraction (20%, tuning data) — never touches `level`, so the
  "floors at the current level" rule holds with no separate clamp needed.
  `reset_xp_on_full_wipe` checks every tick whether every connected player
  is currently `Downed`; if so, zeroes everyone's `xp` — this supersedes
  the individual penalty for whoever was downed last rather than stacking
  with it, matching a wipe being its own outcome. Both run right after
  `death_system` in the server schedule.
- [x] Server-authoritative, persisted per character. Save format: RON
  files under `saves/<game_id>/` (`server::persistence` — `GameSave`,
  `CharacterSave`), reusing the same `Level`/`Stats`/`UnspentStatPoints`
  types the ECS already uses rather than a separate mirror format. Identity
  model (confirmed with the user before implementing, resolving
  `DESIGN.md`'s open question): one server process is one game; a
  client-generated persistent character ID + a game password and a
  character password (both checked server-side, neither remembered
  client-side) carried in netcode's connection-time `user_data` field
  (`protocol::ConnectAuth`) rather than a new gameplay message; a character
  is scoped to one game, not portable across different games; no
  account/login system or lobby UI (real menu is still M8 — see
  `DECISIONS.md`). Duplicate simultaneous connections for the same
  character are rejected; disconnecting a character saves its current
  progression and reconnecting with the right password restores it.

## M6 — Skills
- [x] Skill acquisition and upgrade, data-driven like enemies/items —
  `content::SkillTemplate` (`assets/spells/*.ron`) mirrors
  `EnemyTemplate`'s shape; `game_core::KnownSkills`/`UnspentSkillPoints`
  gate which skills a character can actually cast, persisted like
  `Stats`/`UnspentStatPoints`. Empty for every character right now: there's
  no skill-tree UI to spend points and populate `KnownSkills` yet (that's
  M8's job), so nothing is castable in a live playthrough until then — same
  "build the data model now, gate it behind a later UI" shape M5 used for
  `Stats`. See `DECISIONS.md`.
- [x] At least 2-3 skills with distinct mechanical behavior —
  `game_core::SkillKind`: `PowerStrike` (single nearest target, reuses
  melee-attack targeting), `AoeBurst` (every valid target in radius, not
  just nearest — genuinely different resolution), `SelfBuff` (no target,
  applies an effect to the caster). Content files:
  `power_strike`/`aoe_burst`/`berserk`.
- [x] "Od" resource (regen + combat/action-generated), power attacks that
  consume it — see `MECHANICS.md`. `game_core::Od` (not named `Resource`,
  Bevy's own ECS-resource derive already owns that name); passive regen via
  `tick_od_regen`, bonus gain on any landed melee hit via
  `combat::OD_GAIN_PER_HIT`.

## M7 — Items & forging
- [x] Item drops, pickup, equip — `game_core::item`: `Item`/`Inventory`/
  `Equipment`, server-validated `PickupItemInput`/`EquipItemInput`/
  `UnequipItemInput`. Pickup is a deliberate button-press on the nearest
  drop in range, not automatic walk-over (matches `ReviveInput`'s existing
  interact-button precedent). Equip/unequip are hotkey-driven client-side
  as a stand-in for the real inventory UI (M8), same pattern as M6's skill
  hotkeys.
- [x] Affix/forging system (the "custom system" from `DESIGN.md`) — this
  phrase never actually had a description anywhere in `DESIGN.md` (checked
  its full history; it's been a dangling reference since the initial docs
  commit). Confirmed with the user: a **socket/rune system** — items have
  a fixed number of sockets (from their template), runes are found/
  socketed for permanent stat bonuses, unsocketing is free and reversible.
  `SocketRuneInput`/`UnsocketRuneInput`, `game_core::socket_rune`/
  `unsocket_rune`. See `DECISIONS.md`.
- [x] Loot tables tied to enemy tiers — `LootTable`/`LootEntry` attached
  per enemy instance from `EnemyTemplate::loot_table`/`drop_chance`,
  rolled once at the exact moment of death inside `combat::death_system`
  (see `game_core::roll_loot`).
- [x] Vendor buy/sell economy, individual currency per player — see
  `MECHANICS.md`. Deferred to a part 2 pass: `MECHANICS.md` itself calls
  this "a distinct system" from the loot/forging pipeline above. Currency
  is built once here and shared by both vendors and forging cost (see
  `DECISIONS.md`'s M7 part 2 planning entry): a currency drop is another
  weighted `LootTable`/`LootEntry` entry, not a separate guaranteed-per-
  kill mechanism. Socketing (M7 part 1's `socket_rune`) will gain a
  currency cost; unsocketing stays free/reversible as shipped. Exact
  amounts/scaling deliberately not decided yet.
  - [x] Currency foundation shipped: `game_core::economy::Currency(u32)`,
    individual per player, replicated. `DroppedLoot`/`LootKind` gained a
    `Currency(u32)` variant — a fixed amount per weighted entry, not a
    random range (payout variance comes from adding several `Currency`
    entries at different weights, same mechanism as item/rune odds, not
    a second layer of randomness). Persisted in `CharacterSave` without a
    `#[serde(default)]` — matches the precedent every earlier field
    added to that struct already set (old saves regenerate rather than
    migrate, an accepted cost during active development). HUD shows a
    "Coins: N" line. Confirmed live: killing enemies with the new
    `Currency` loot entries (`converted_farmer`/`missionary`) and
    picking them up increases the HUD count.
  - [x] Socketing-cost wiring shipped: `RuneDefinition`/`content::RuneTemplate`
    gain a required `socket_cost: u32` (no default — same "a content
    author always makes a conscious choice" convention as
    `EnemyTemplate::xp_reward`, so a new rune can't silently become free
    to socket by omission). `socket_rune` now takes `&mut Currency`,
    rejecting (no state change at all, including no currency deducted)
    if the balance can't cover it — `unsocket_rune` is untouched, still
    free/reversible. Client's forging panel shows a live "Coins: N" line
    and each rune button as `"<rune_id> (xN) - <cost>g"`, disabled via
    `egui::Button`/`add_enabled` when unaffordable — a new client-local
    `RuneTemplates` resource (mirroring `EnemyTemplates`) exists purely
    for this display/gating; the server independently enforces the real
    charge regardless of what the client shows. `crit_shard`/
    `swift_shard` priced at 20/15 — placeholder numbers, not tuned (see
    `MECHANICS.md`'s open questions on currency amounts). `spawn_starter_loot`
    gained a 50-currency drop alongside the existing weapon/runes so
    socketing is testable without an enemy kill first. Confirmed live:
    unaffordable rune buttons greyed out, a successful socket deducted
    coins, unsocketing gave no refund.
  - [x] Vendor buy/sell UI and content schema shipped. `Interactable`'s
    single `opens_panel: Option<String>` generalized to
    `opens_panels: Vec<String>` — one NPC can now offer more than one
    capability (a blacksmith is both `"forging"` and `"vendor"`), with `E`
    opening every panel the nearest interactable declares at once (falling
    back to `dialog` only if none apply), and closing all of them together
    on a second press rather than one at a time. New `game_core::economy`:
    `VendorListing`/`VendorLibrary`, `buy_item` (charges a vendor's listed
    price, sizes the new item's sockets from its template), `sell_item` +
    shared `socketed_item_sell_value` (an item's base `ItemDefinition::
    sell_value` plus each socketed rune's own `socket_cost` — reused as
    the rune's implicit worth, since runes still can't be bought/sold on
    their own; break-even by design, socketing then immediately reselling
    nets back exactly what the socketing cost). `ItemDefinition`/
    `content::ItemTemplate` gain a required `sell_value: u32` (same
    "conscious choice" convention as `xp_reward`/`socket_cost`). New
    `content::economy::VendorTemplate` (`assets/vendors/*.ron`), keyed by
    the same `template_key` as its `Interactable` placement — a
    `blacksmith.ron` (rusty/steel swords) and a new `merchant.ron`
    (armor/helmet), the merchant being a vendor-only NPC placed at a third
    map point purely from content, no new mechanism. Two new `protocol`
    messages, `BuyItemInput`/`SellItemInput` — neither carries *which*
    vendor; the server re-resolves the nearest vendor-panel `Interactable`
    itself from the actor's position (`nearest_interactable_with_panel`,
    replacing the old bool-returning `is_near_interactable_with_panel`),
    never trusting a client-claimed target. Client's vendor panel requires
    an explicit Confirm/Cancel step before a sell actually fires (the
    user's call — selling is destructive, a socketed item's runes are
    lost with no refund) but buys immediately (not destructive). New
    client-local `ItemTemplates`/`VendorTemplates` resources purely for
    the buy-listing/sell-price-preview display, reusing
    `socketed_item_sell_value` directly rather than re-deriving the
    formula. `spawn_starter_loot`'s currency bumped 50→150 to cover both a
    socket action and the blacksmith's pricier `steel_sword` listing.
    **Confirmed live:** blacksmith opens both panels together; bought
    `steel_sword` and watched coins/inventory update; sold an item via the
    confirm flow (and separately confirmed Cancel does nothing); merchant
    opens vendor-only with its own distinct stock; `E` closes every open
    panel at once regardless of how many were open.
    Weapon-power scaling itself (vendors selling mechanically stronger
    weapons, not just more sockets) is explicitly out of scope — blocked
    on the same not-yet-built "weapon-driven combat stats" prerequisite
    `DECISIONS.md` already flagged for M8's deferred hotbar/weapon-swap
    items.
- [ ] Enemy visual-variant data shape (shared base template + swappable
  sprite field) — see `MECHANICS.md`. Deferred to the same part 2 pass;
  unrelated to forging, just filed under the same milestone.

Item/rune stat bonuses (crit chance/multiplier, move speed) are wired into
live combat/movement resolution this pass, computed fresh at point of use
(`Equipment::stat_bonus`) — not left inert like M5's `Stats` bonuses, to
avoid stacking up two unwired mechanical systems. `Stat` deliberately has
no `MaxHealth` variant yet (see `DECISIONS.md`): safely rescaling current
health when max changes needs its own care, not worth rushing into this
pass just to add a rune type for it.

## M8 — UI: HUD & menus

Part 1 (done — see `DECISIONS.md`'s M8 planning entry for the full design
writeup). Numbered here by practical build order, not importance — later
steps depend on earlier ones landing first; a review pass before starting
found two gaps folded in below (marked "found via review").

1. [x] `bevy_egui` added as a client-only dependency (named in `CLAUDE.md`'s
  stack from the start; `=0.41.1`, verified compatible with the pinned
  `bevy = "=0.19.0"`). `EguiPlugin::default()` wired into `client`'s
  `App`, no panels yet — confirmed the server's dependency tree stays
  clean (`cargo tree -p server` has no `egui`).
2. [x] Backend prep, no UI yet — makes the panels below meaningful the
  moment they exist instead of needing a follow-up fix:
   - Wire `Stats`' `bonus_move_speed`/`bonus_crit_chance`/
     `bonus_crit_multiplier` into combat/movement resolution, the same
     additive-at-point-of-use way `Equipment`'s bonuses already are.
     `bonus_max_health` stays deferred, same reason as item-based
     max-health bonuses: safely rescaling current health when max
     changes needs its own care. **Found via review:** `Stats`' bonus
     fields were completely unread anywhere in the codebase before this —
     the stat-allocation panel below would otherwise let a player spend
     points for zero gameplay effect, the exact gap M7 deliberately
     avoided for item/rune bonuses. Wired into `combat::attack_system`'s
     effective crit stats and `server`'s `apply_move_input`'s effective
     speed, both computed fresh at point of use like `Equipment`'s
     bonuses — never baked into the base component.
   - Replicate `AttackTimer`/`SkillCooldowns` (both server-only until
     now) so the HUD can show real cooldown countdowns. Both gained
     `Serialize`/`Deserialize` and a `.replicate::<T>()` registration in
     `protocol`.
3. [x] egui HUD: health/od bars, skill cooldowns, downed-state indicator.
  Skill icons use the existing fixed `1`-`3` hotkeys from M6, no new
  input model — a text row per hotkey (id + remaining cooldown or
  "ready"), not real icon art. Read-only — first panel, no input-focus
  handling needed yet. Confirmed live: a two-terminal server+client
  playtest, screenshotting the actual window, showed the panel
  correctly rendering replicated `Health`/`Od` and an empty
  `SkillCooldowns` (all three hotkeys "ready") over the running game
  world. Didn't get a live screenshot of the downed-indicator branch
  specifically (simulating a keypress to force it hit a macOS
  Accessibility-permission wall mid-session) — that branch is a
  one-line `if downed`, the same `Has<Downed>` pattern already proven
  live elsewhere in `client` (`player_appearance_system`), not new
  logic of its own.
4. [x] Input-focus guard, built alongside step 5's panel below.
  `bevy_egui` 0.41's `EguiPlugin` already runs `write_egui_wants_input_system`
  by default, populating `Res<bevy_egui::input::EguiWantsInput>` every
  frame with no extra setup — confirmed by reading the crate source, not
  assumed. `player_input_system` reads `wants_keyboard_input()` (real
  `TextEdit`-style focus, not just the mouse hovering a panel — hovering
  alone would make the toggle key unable to close the very panel the
  cursor is resting on) and gates every hotkey below the check on it.
  `WASD`/`Space`/`F`(revive)/`1`-`3` stay above the check, never gated,
  per `CLAUDE.md`. The panel below has no text field, so this guard has
  no observable effect yet — correctly-wired infrastructure for when one
  exists, not a behavior change today.
5. [x] Inventory + equip/unequip panel — click-driven, replaces M7's
  `F1`-`F6` hotkey stand-ins (removed, not kept alongside it).
  `INVENTORY_TOGGLE_KEY` (`I`) opens/closes an egui window (also
  closable via its own close button, both driving the same
  `InventoryOpen` flag) listing equipped slots (with an Unequip button
  per occupied slot) and inventory items (with an Equip button each),
  sending the same `EquipItemInput`/`UnequipItemInput` messages the old
  hotkeys sent — server-side resolution (`apply_equip_input`/
  `apply_unequip_input`) is unchanged. Items display by raw
  `template_key` (e.g. `"rusty_sword"`); `ItemTemplate` has no display-
  name field yet. Confirmed live: a server+client playtest showed the
  toggle opening/closing the panel, the empty-inventory/equipment state
  rendering correctly, and WASD movement plus combat continuing to work
  normally while the panel stayed open (proving the guard doesn't
  suppress the protected keys). Didn't get a live click-through of the
  Equip button itself at the time (simulating mouse/keyboard input to
  drive the external game window twice caused the *host* terminal to
  lose/regain focus unexpectedly, once nearly disrupting the developer's
  own session) — that path leaned on `game_core::item::equip_item`'s
  existing unit tests plus the unchanged server handlers. **Update:**
  when the user manually tested it afterward, Equip didn't work at all —
  a real bug this pass's screenshot-only verification couldn't have
  caught. See `DECISIONS.md`'s "`bevy_egui` 0.41 panels render but don't
  accept clicks unless drawn from `EguiPrimaryContextPass`" entry for the
  root cause and fix (all three egui panel systems were in the wrong
  schedule); confirmed fixed by the same user re-testing live.
6. [x] Level-up / stat-allocation panel (new `AllocateStatPointInput`
  message — `UnspentStatPoints` has had nowhere to go since M5) and a
  skill-learning panel (new `LearnSkillInput` message — same gap for
  `UnspentSkillPoints`/`KnownSkills` since M6), combined into one
  `C`-toggled "Character" window. Meaningful from the moment it exists
  since step 2 already wired the stat bonuses in. Flat spend-a-point
  list, no prerequisite tree topology — nothing in `MECHANICS.md`/
  `DESIGN.md` specifies an actual tree shape; learning an already-known
  skill again just increments its level (`KnownSkills`' existing "starts
  at 1, no unranked state" shape).
  `game_core::allocate_stat_point`/`learn_skill` do the actual spend
  (unit-tested: 3 tests each, covering the reject-when-empty and
  accumulate-across-spends cases); `Stat` (already the type
  `RuneDefinition`/`StatModifier` use) gained `Serialize` so it could
  become `AllocateStatPointInput`'s payload — no new parallel enum
  needed, and its match in `allocate_stat_point` is exhaustive without a
  `MaxHealth` case since `Stat` itself has none (see its doc comment for
  why that stat stays deferred). `UnspentStatPoints`/`UnspentSkillPoints`
  are now replicated (previously weren't) so the panel can show real
  counts and disable a button once a stat/skill has no point left to
  spend. Confirmed live: a fresh character's panel showed `Level 1 (XP
  0/100)`, all three stat rows at `+0.00` with a disabled `+1`, and all
  three skills at `level 0` with a disabled `Learn` — correct given zero
  points — plus the `C` toggle cleanly opening and closing it. Didn't
  farm the ~10 kills needed to reach a real level-up and exercise the
  enabled/clickable path live, for the same reason M8 step 5 stopped
  short of live-clicking Equip: the input-simulation risk to the
  developer's own session wasn't worth it given `allocate_stat_point`/
  `learn_skill` are already directly unit-tested. Also affected by, and
  fixed by, the `EguiPrimaryContextPass` schedule fix noted under step 5
  above — this panel's buttons were subject to the exact same bug, just
  not yet caught live here since none were enabled (0 points) to click.
7. [x] Generic `Interactable` system (`game_core::interact`): proximity +
  action button (`E`, same key as pickup — checks interactables first,
  falls back to nearest item drop). **Corrected via review:** replicates
  as `Interactable { template_key: String, range: f32 }` only — the same
  "replicate a content-template key, not template data" pattern as
  `EnemyKind`, not an embedded `EffectDefinition` (which doesn't derive
  `Serialize`/`Deserialize` and is otherwise server-only). The client
  loads `InteractableTemplate`s locally for dialog text (same pattern as
  `EnemyTemplates`); the server resolves any effect grant via a new
  `InteractableLibrary` lookup, applied unconditionally to the
  interacting player (`EffectDefinition::applies_to` doesn't apply here
  — there's no "attacker"). Pulls forward part of M9's "special-character
  dialog" mechanism by necessity; only the generic trigger, not M9's
  actual objective content.
  The priority resolution (`interact_or_pickup_system`) moved the pickup
  logic itself from `server` into `game_core` for the first time — same
  "server writes an event carrying who acted, `game_core` resolves it"
  split as `attack_system`/`skill_cast_system`, unit-tested (6 tests:
  nearest-interactable priority, out-of-range fallback, multiple
  interactables, unknown template key still taking priority over
  pickup). `PICKUP_RANGE` moved alongside it (was a plain `server`
  constant). No new client message: `PickupItemInput` now translates to
  `InteractOrPickupRequested` server-side, one button still doing double
  duty. `InteractableLibrary`/client `InteractableTemplates` both load
  for real from `assets/interactables/` (not a `Default`-empty
  placeholder) — the directory exists but is genuinely empty today (no
  `.ron` files yet, just a `.gitkeep`), which content-loading already
  treats as valid (only a *missing* directory is a load error), so both
  client and server start up the same way real content will load once
  M8 step 8 adds it — confirmed live, both processes start without
  panicking. Didn't get a full live "kill enemy → verify item stays on
  the ground until `E`" round trip: three separate synthetic-input
  focus-flips onto the developer's own terminal during this session (one
  nearly polluting a live prompt) made further automated play-testing not
  worth the risk. The exact resolution logic this would have exercised
  is what the 6 unit tests above directly cover.
8. [x] Interactables (blacksmith, runestones) placed via a new Tiled
  object layer in `assets/maps/valley.tmx` (named point objects, read
  the same way `spawn_map_colliders` already reads the collision layer)
  — the map is the source of truth for placement, not a hardcoded spawn
  constant. Hand-edited into the TMX for now since the user doesn't have
  the Tiled editor installed yet. No explicit "Neutral" marker needed
  (confirmed via review): not giving these entities a `Health` component
  already excludes them from every combat-targeting query (`ai_system`
  only ever targets `With<Player>`; melee/skill targeting requires
  `With<Health>`).
  `server::spawn_interactables` (new Startup system) reads the
  "interactables" object layer's named `Point` objects, matches each
  name against a `content::InteractableTemplate` key, and spawns
  `Interactable { template_key, range }` — loading templates directly
  (mirroring `spawn_enemies`) rather than via `Res<InteractableLibrary>`,
  since `range` is spawn-time-only data (see that resource's doc
  comment). A name with no matching template panics at Startup, same
  "fail loudly" convention as `spawn_enemies`. Two real content files now
  exist and are placed: `runestone` (a `CritChance` `StatModifier` buff)
  and `blacksmith` (no effect, `opens_panel: Some("forging")` — inert
  until step 10). Client renders both with a placeholder blue-violet
  square (`init_replicated_interactables`), same "render, don't
  simulate" split as enemies. **Found via live testing, not assumed:**
  `ObjectShape::Point`'s tuple fields are deprecated in `tiled` `0.16.0`
  (superseded by the object's own `x`/`y`) — matching `Point(..)` instead
  of `Point(x, y)` avoids a clippy `-D warnings` failure that would
  otherwise block the build. Confirmed live end-to-end with temporary
  debug logging (removed before commit): walking up to the blacksmith
  correctly found it as nearest and correctly applied no effect; walking
  to the runestone correctly found *it* as nearest instead and logged
  `applying effect "runestone_blessing"`. The user's first two attempts
  read as "E does nothing" — both were actually correct behavior (too
  far from either object, then standing at the effect-less blacksmith)
  rather than a bug; only found the *actual* correct interpretation by
  adding temporary instrumentation rather than guessing further.
9. [x] Dialog panel (generic text window) for runestone-style
  interactions — the simpler of the two interaction outcomes, built
  first to validate the whole `Interactable` pipeline end-to-end before
  adding forging's extra gating logic below.
  Extracted `game_core::nearest_interactable_in_range` out of
  `interact_or_pickup_system` so the client's dialog trigger and the
  server's effect resolution share one priority rule instead of the
  client re-implementing it — matters because the dialog has to open
  instantly, client-side, with no server round-trip (per `DECISIONS.md`'s
  M8 planning entry). New `client::DialogPanel` resource +
  `dialog_trigger_system`/`dialog_panel_system`, rendered from
  `EguiPrimaryContextPass` like the other panels. **Confirmed live, then
  refined from live feedback:** the first pass closed the dialog only via
  egui's own mouse-driven close button; testing it revealed that's
  awkward when the dialog pops up mid-fight (e.g. picking up loot in
  range of a dialog `Interactable`), so `PICKUP_KEY` (`E`) now toggles —
  a second press closes the panel immediately instead of re-checking
  proximity, without touching the underlying interact/pickup request
  (still fires every press, unchanged). Both existing interactables
  (`runestone`, `blacksmith`) already had `dialog` text from step 8, so
  no new content was needed to test this.
10. [x] Forging UI, triggered by a blacksmith-kind `Interactable` —
  sockets/unsockets now require being in range of that specific NPC (a
  real behavior change from M7's free-anywhere hotkey socketing);
  `apply_socket_rune_input`/`apply_unsocket_rune_input` gain a
  server-side proximity check, not just a plain range constant.
  New `game_core::is_near_interactable_with_panel` + `FORGING_PANEL_ID`
  shared constant: same "one helper, not duplicated logic" precedent as
  step 9's `nearest_interactable_in_range`, checking *any* in-range
  `Interactable` whose library definition has the matching `opens_panel`
  (not just the nearest one overall — a player near both a runestone and
  the blacksmith should still be able to forge). `client`'s `DialogPanel`
  generalized into `InteractionPanel` (`Dialog(String)` or `Forging`),
  since one button/one trigger system now resolves to at most one of the
  two. New `forging_panel_system` lists each equipped slot's sockets;
  empty ones offer one button per rune actually in the player's
  `RuneInventory` (read live off replicated data, not a hardcoded rune
  list) — removed the `F7`/`F8`/`F9` hotkey stand-ins now that the panel
  covers the same ground, same precedent as the inventory panel replacing
  `F1`-`F6`. **Confirmed live:** equip a socketed weapon, open the panel
  at the blacksmith, socket and unsocket a rune, and confirm socketing is
  silently rejected out of range. Testing surfaced two gaps addressed
  along the way, both live-tested themselves: enemies weren't dropping
  loot to test with, so `server::spawn_starter_loot` seeds a socketed
  `rusty_sword` + one `crit_shard`/`swift_shard` rune near the map
  midpoint between the runestone and blacksmith — the user asked to keep
  this permanently rather than strip it as a one-off test aid, but only
  once per new game (gated on the same new-vs-existing-game save check
  `load_game_password` already makes), not every restart, since an
  unconditional Startup spawn would otherwise pile up duplicate loot on
  every server restart of an already-running game.

Deferred (see `DECISIONS.md` for why):
- [ ] `TAB`-style toggle-through skill/attack selector, dedicated
  per-attack hotkeys, primary/secondary weapon slots — blocked on
  weapon-driven combat stats not existing yet, not a UI-only gap.
- [ ] Player skin preset selection; equipped armor/helmet renders visually
  (full per-item outfit changes — see `MECHANICS.md`). Blocked on actual
  art assets, not just code — everything's solid-color placeholder
  sprites today.

Minimap and full party status detail are not blocked on anything — see
M8.5 below, which builds them alongside the lighting/ambience foundation.

## M8.5 — Lighting & ambience foundation

A prerequisite for real art (maps, tiles, character sprites): lighting and
ambience effects need to exist and be tunable *before* colors get chosen
for actual sprites, not guessed at blind. Also closes the one real
still-unbuilt M8 UI gap (minimap, party status) while here. Everything in
this milestone is client-only rendering/presentation — no `game_core`/
`server`/`protocol` changes, per `CLAUDE.md`'s crate boundaries (lights and
particles have no gameplay effect to authorize or keep in sync).

- [x] Minimap + full party status detail — HP/Od bars and downed/stunned
  indicators for every party member (local + remote), plus a small
  `egui::Painter` minimap plotting positions against the map's known fixed
  extent. All the underlying data (`Health`/`Od`/`Downed`/`Stunned`) is
  already replicated; no new protocol work — new `PartyStatus` query type
  (kept separate from `PartyPositions` since `party_centroid_and_spread`
  assumes that one yields a bare `&Position`), `party_status_system` and
  `minimap_system`. Confirmed live: minimap dot and HP/Od bars render and
  track a moving player.
- [ ] `bevy_lit` added (client-only) for 2D dynamic lighting: ambient
  light, point lights, shadow-casting occluders.
  - [x] `bevy_lit` `=0.11.0` added to `client` only (verified the exact
    resolvable version via `cargo add --dry-run` rather than trusting
    research alone; `cargo tree -p server` confirmed no `bevy_lit`/
    `bevy_render` growth on the server). `Lighting2dPlugin` wired,
    `Lighting2dSettings`/`AmbientLight2d` (both defaults) attached to the
    `Camera2d`. One `PointLight2d` placed at the player's spawn point as a
    deliberate cheap smoke test for the untested `bevy_lit`+`bevy_egui`
    combination, before investing further. **Confirmed live**: scene
    renders correctly with a visible glow, and — the actual point of the
    test — every egui panel stayed fully clickable (Equip, character
    panel, Party window all clicked with the light active), not just
    visible. This is the same class of bug M8's `EguiPrimaryContextPass`
    issue was; a screenshot alone would not have caught a regression here.
  - [x] First shadow-casting occluder: `cast_shadows: true` on the test
    light, plus one placeholder "pillar" (`Mesh2d`/`ColorMaterial`
    rectangle) with `LightOccluder2d::default()` (shape-based occlusion —
    the default empty `occluder_mask` falls back to the attached mesh's
    own shape). Deliberately capped at one occluder given the open
    upstream perf issue — measured a real frame time before considering
    more: a temporary `FrameTimeDiagnosticsPlugin`/`LogDiagnosticsPlugin`
    (added, measured, then removed — not left in the codebase) logged a
    stable ~60 FPS / ~16.7ms frame time with the light, ambient light, and
    occluder all active together, no concern at this count. **Confirmed
    live**: the pillar casts a correctly-shaped shadow away from the light.
- [x] Status indicators (downed/stunned/leash-warning) redesigned off
  `Sprite.color` onto a gizmo overlay, so the base sprite color is free
  for the lighting system (and later, real textures) to own. Old
  `player_appearance_system`/`PartySprites` removed entirely (base color
  was already set once at spawn in `init_replicated_players`/
  `init_replicated_enemies` — the sprite-recolor pass existed purely for
  status, nothing else needed it); new `status_indicator_system` draws a
  colored ring (`gizmos.circle_2d`) with the exact same `Downed` >
  `Stunned` > leash-warning priority the old code used, immediate-mode
  like `facing_indicator_system` — no spawned entity, no stale-state risk.
  **Confirmed live**: player sprite stays its normal color at all times;
  a yellow ring appears around a stunned player and disappears once the
  stun wears off.
- [x] `bevy_hanabi` added (client-only) for GPU particle effects: sparks/
  glow at torches, placed via a new Tiled object layer, read purely
  client-side (no server involvement — cosmetic only).
  - [x] Torch placement + lighting shipped: new `"ambience"` Tiled object
    layer in `assets/maps/valley.tmx` with two named `"torch"` point
    objects near the runestone/blacksmith — same hand-edit-the-TMX
    approach M8 step 8 used. New `spawn_torch_lights`, a client-only
    system reading `bevy_ecs_tiled`'s `TiledEvent<ObjectCreated>` and
    inserting a warm `PointLight2d` directly onto the Tiled-spawned
    object entity — no separate entity spawn, no server/protocol/
    `Replicated` involvement at all (purely cosmetic, same category as
    the map's client-only visual tile layers). **Two real findings from
    live debugging, not assumed:**
    - The anticipated risk — that `bevy_ecs_tiled`'s own per-object
      `Transform` might use a different coordinate convention than the
      project's `world_y = -tiled_y` rule — did **not** materialize.
      Empirically confirmed via a temporary debug print that it lands at
      exactly `(tiled_x, -tiled_y)` under `TilemapAnchor::TopLeft`,
      matching the server's manual convention precisely. No fallback to
      a second `tiled::Loader` pass was needed.
    - The actual bug: `bevy_ecs_tiled` sets the object's `Name` component
      to a wrapped `"Point(torch)"`-style string (shape kind included),
      not the raw Tiled name — an exact-match against `Name` silently
      matched nothing. Fixed by matching against the separate `TiledName`
      component instead, which holds the plain `"torch"` string. Found
      via a live debug print after the first attempt rendered nothing,
      not guessed at.
  - [x] `bevy_hanabi` `=0.19.0` added to `client` only, `default-features =
    false, features = ["2d"]` (the project has no 3D rendering; upstream
    defaults are 3D-oriented) — resolvable version verified via `cargo add
    --dry-run --no-default-features --features 2d` before pinning, `cargo
    tree -p server` confirmed no leak. `HanabiPlugin` registered; a shared
    `torch_spark` `EffectAsset` (radial-drift spark/glow, modeled on
    `bevy_hanabi`'s own `examples/2d.rs` shape rather than a hand-authored
    directional "rising ember" — not worth the extra `ExprWriter`
    complexity for this placeholder pass) built once in `setup_scene` and
    stored in a `TorchSparkEffect` resource, attached via
    `ParticleEffect::new(...)` to each torch entity alongside its
    `PointLight2d` in `spawn_torch_lights`. One incidental fix: `Gradient`
    is ambiguous between `bevy::ui::Gradient` and `bevy_hanabi::Gradient`
    with both preludes in scope — resolved by fully qualifying
    `bevy_hanabi::Gradient`. **Confirmed live**: warm sparks trickle
    outward from both torches alongside the existing glow; all egui panels
    remained fully clickable with both `bevy_lit` and `bevy_hanabi` active
    together.
- [x] A lighting/ambience debug panel: sliders for ambient and point-light
  color/intensity/radius — the actual sandbox for picking sprite/tile
  colors against real lighting before any art exists. New
  `lighting_debug_panel_system` (`EguiPrimaryContextPass`, anchored
  bottom-right): one section for the single `AmbientLight2d` (queried via
  `Single`), plus one collapsible section per `PointLight2d` entity,
  labeled by `TiledName` where the light came from a Tiled object (the
  torches) and by entity id otherwise (the smoke-test light). Color edited
  via `egui::color_edit_button_rgb`, round-tripped through
  `Color::to_srgba`/`Color::srgb`. Dev-only tool: nothing persisted or
  replicated, values reset on restart. One real bug found live: both
  torches share the `TiledName` `"torch"`, so using that label directly as
  the collapsing section's egui id caused a widget-id clash (egui's own
  "first/second use of widget ID" warning overlay, which visually blocked
  the second torch's sliders) — fixed by wrapping each light's whole
  widget block in `ui.push_id(entity, ...)` so same-named lights get
  distinct ids. **Confirmed live**: ambient and point-light sliders all
  work, both torch sections expand independently with no id-clash warning.

**Definition of done, confirmed live:** placeholder sprites + the real
Tiled tilemap render correctly with `Lighting2dPlugin` active; the pillar
occluder casts a correct shadow; `AmbientLight2d` is live-adjustable via
the debug panel; both torch particle effects render alongside everything
else; every egui panel (HUD, inventory, character, dialog, forging,
vendor, party status, minimap, lighting debug) is fully click-interactive;
a temporary `FrameTimeDiagnosticsPlugin` recorded a stable ~60 FPS with
lighting + the occluder + both torch lights + particles + the debug panel
all active together, matching the earlier occluder-only measurement — see
`DECISIONS.md`'s M8.5 entry for the full writeup.

## M8.6 — Weapon-driven combat

Client/server split as usual — weapon stats and timing are simulation state
(server-authoritative, `game_core`), presentation (HUD countdown) is
`client`. See `MECHANICS.md`'s Weapons & attack timing section for the full
mechanic shape this builds; the below is the build breakdown.

**Implementation status: everything below except the HUD bullet is built,
unit-tested, and passes the full verification loop (build/test/clippy/fmt).**
Built autonomously (the user was away from their computer), then
**confirmed live** afterward: a real server+client session, player attack
resolution instrumented with temporary `tracing::debug!` logging (see the
new `DECISIONS.md` logging entry) to verify hits actually landed and dealt
damage, since this pass had no live coverage at build time. The user
reported combat working (cone-gating/windup made a kill "difficult" but
achievable — first live signal on how the new timing actually feels, not
yet tuned against), plus pickups and dialogs (pre-existing M8 features,
unaffected by this milestone) still working correctly. This was a
single-client session, not a two-client co-op verification — the
enemy-facing-snap and cone-gating haven't specifically been exercised
against a second player's perspective yet. See `DECISIONS.md`'s M8.6 entry
for the full design writeup, the decisions confirmed with the user before
implementing, and known limitations.

- [x] Weapon content schema: `ItemDefinition` gains `weapon:
  Option<WeaponStats>` (`damage`/`damage_type`/`range`/`attack_duration`/
  `recovery`, required and exclusively for `slot: Weapon` items — enforced
  at content-load time via a new `ContentError::Validation`, not just a
  convention) — resolves `MECHANICS.md`'s long-standing "ranges are content
  data, eventually" note for real.
- [x] Phased attack timing — player-only (see below), replacing
  `MeleeAttack::cooldown`'s flat number with a new `game_core::AttackPhase`
  enum (`Idle | Windup{remaining, target} | Recovery{remaining}`) and pure
  `tick_attack_phase` transition function, in a new `game_core::
  weapon_attack` module: windup → damage resolves → recovery lockout. A
  windup in progress is cancelled outright (no hit, straight back to
  `Idle`, not even entering `Recovery`) if the attacker is
  `Stunned`/`Downed` when its tick runs — enemies have no use for this
  since they never enter `AttackPhase` at all (see below), but a player who
  dies mid-windup already can't act via the same mechanism.
- [x] Rooted during windup, free to move during recovery — `server`'s
  `apply_move_input` zeroes velocity while `AttackPhase::Windup`, flagged
  in its doc comment as a starting assumption per `MECHANICS.md`, not
  confirmed live.
- [x] Derive the player's effective attack from their equipped weapon,
  computed fresh from `Equipment` + `ItemLibrary` every attack (never
  cached) via `game_core::effective_weapon_stats`, replacing the hardcoded
  `MeleeAttack` previously spawned in `server/src/main.rs` (its
  `PLAYER_ATTACK_*`/`PLAYER_FURY_*` constants removed entirely). Both the
  damage sum (crit stats) and the resistance sum are built open-ended, as
  requested — `resolve_melee_hit`'s effective-`CombatStats`/effective-
  `Resistances` construction stays a plain sum of named terms, ready for
  M8.7/M8.12 to add more without restructuring.
  **Found via review, fixed before this could ship as a silent bug:** the
  first pass built `Equipment::resistance_bonus` and wired
  `Resistances::default()` onto the player spawn, but never actually
  called `resistance_bonus` from damage resolution — exactly the "stat
  exists but does nothing" class MECHANICS.md's "Effective combat values
  are always computed fresh" section warns about by name. Caught while
  writing this changelog entry, not by a test failure; fixed by threading
  `target_equipment`/`ItemLibrary` into `resolve_melee_hit` and adding
  `attack_system_applies_the_targets_equipped_armor_resistance`, which
  would have failed had the bug still been present.
- [x] **`resolve_melee_hit`, a new shared helper factored out of
  `attack_system`'s hit-resolution body**, used by both the enemy path
  (`AttackTimer`, unchanged behavior — the full existing test suite passes
  byte-for-byte after the extraction) and the new player path
  (`AttackPhase`), so the two attacker-timing models can't silently diverge
  on crit/resistance/XP/Od/effect resolution. A second real gap found the
  same way as the resistance one: the player path's first draft passed a
  throwaway empty `RuneLibrary::default()` into this helper instead of the
  real resource, which would have silently disabled socketed-rune crit
  bonuses on player attacks specifically (enemies never socket runes, so
  this wouldn't have shown up there) — fixed alongside it.
- [x] Also reads socketed runes on the equipped weapon/armor into the
  effective-attack computation, via the existing `Equipment::stat_bonus`
  plumbing `resolve_melee_hit` already calls — no rune grants bonus damage
  or an on-hit effect yet (that's M8.12), so there's nothing new to
  observe from this bullet specifically yet, but the read path is real and
  exercised by existing socketed-rune crit tests.
- [x] Unarmed fallback: `game_core::unarmed_weapon_stats()`, a single named
  function (not inline defaults duplicated at each call site) that
  `effective_weapon_stats` falls back to for an empty `Weapon` slot or an
  equipped item whose template is unknown to `ItemLibrary`.
- [x] Armor/Helmet gain a real mechanical effect: `ItemDefinition` gains
  `resistances: Resistances` (enforced empty for `Weapon` items at
  content-load time, same validation as the `weapon` field), summed across
  equipped slots via `Equipment::resistance_bonus`. Players gain a base
  `Resistances::default()` component on spawn, matching enemies.
- [x] Real weapon `.ron` archetypes: `rusty_sword` (fast/light — 8 damage,
  55 range, 0.25s windup, 0.4s recovery) and `steel_sword` (slow/heavy —
  16 damage, 65 range, 0.4s windup, 0.65s recovery), both in
  `assets/items/`. `leather_armor`/`wolf_pelt_cap` each gained a modest
  `primal` resistance. All four numbers are tuning data, not final.
- [x] Directional (facing-cone) melee arcs: `game_core::combat::
  is_within_attack_arc` (a single fixed half-angle,
  `MELEE_ARC_HALF_ANGLE_RADIANS`, shared by every weapon for now — not yet
  a per-weapon field), wired into both `attack_system`'s nearest-target
  search (enemies) and the new `weapon_attack::find_attack_target`
  (players). **User's explicit call, against the initial recommendation**
  (see `DECISIONS.md`): both players *and* enemies are cone-gated, not
  players only — which in turn required a real behavior addition beyond
  the original scope of this bullet, `game_core::enemy::ai_system` now
  snaps an attacking enemy's `Facing` directly at its target the instant
  it issues that attack, overriding whatever stale movement-derived facing
  is left over from its chase (see `MECHANICS.md`'s Facing section for the
  documented exception this carves out of the "purely movement-derived"
  rule that's held since M4). Existing `attack_system` tests needed
  `Facing` added to their attacker fixtures — every existing test target
  happened to already sit along the attacker's `+X` axis, so this was a
  mechanical fixture update, not a rewrite of test intent; two new
  `attack_system` tests (`_whiffs_an_in_range_target_outside_the_facing_cone`
  / `_hits_an_in_range_target_inside_the_facing_cone`) exercise the cone
  itself directly, since the existing fixtures alone would have kept
  passing even if the cone filter were silently broken.
  Deliberately **out of scope, flagged not guessed**: cone-gating enemies
  required this facing-snap, which was *not* originally part of this
  bullet — surfaced as a required consequence during planning, confirmed
  with the user, and folded in rather than silently decided.
- [x] Tests: `game_core` gained ~30 new/changed tests across `combat.rs`
  (cone math, `resolve_melee_hit`/`AttackerProgress` extraction, target
  resistance), the new `weapon_attack.rs` module (phase transitions —
  windup/recovery/cancellation/incapacitation, `effective_weapon_stats`
  fallback chain, `find_attack_target`, both new systems via
  `run_system_once`), `item.rs` (`Equipment::resistance_bonus`), and
  `enemy.rs` (attack-time facing snap). `content::item` gained schema
  validation tests (weapon-without-block, non-weapon-with-block,
  weapon-with-resistances, all rejected; a well-formed pair accepted).
- [ ] HUD: extend the existing attack-cooldown display to show windup vs.
  recovery distinctly rather than one flat countdown. **Not done this
  pass** — `AttackPhase` is deliberately not yet replicated (server-only
  resolution state, mirroring `ActiveEffects`); a small replicated summary
  for this display is the natural next sub-pass once this milestone is
  live-tested.

**A decision made explicitly, not assumed — phasing is player-only.**
Enemies keep `AttackTimer`'s flat-cooldown resolution completely unchanged;
`attack_system` (still the enemy-only resolution path now that players no
longer carry `MeleeAttack`/`AttackTimer` at all) is otherwise untouched
apart from the cone-gate addition above. Extending phased windup/recovery
to enemies — which would need new `EnemyTemplate` content fields and an AI
"committed to windup" behavior — is unblocked but not built here; see
`DECISIONS.md`'s M8.6 entry for why this was scoped out rather than
attempted alongside everything else in this pass.

Deliberately deferred, unblocked but not built here: weapon *switching*
(primary/secondary slots, the M8 TAB-selector deferral) — single-weapon
combat should be solid first.

## M8.7 — Primary attributes & attack speed (revises M5)

**Implementation status: everything below is built, unit-tested, passes the
full verification loop (build/test/clippy/fmt), and confirmed live** —
server started with `RUST_LOG=info,game_core=debug`, client connected,
combat confirmed feeling fine (no specifics flagged beyond that). Built in
a session with the user present for a decisions round (`AskUserQuestion`)
on the two shapes `MECHANICS.md` didn't already settle. **Not yet
committed** — still uncommitted local changes as of this playtest.

- [x] `game_core::progression`: introduced an `Attribute` enum (Might,
  Dexterity, Vitality, Intelligence) as the new target for
  `allocate_stat_point`'s spend, replacing direct-to-secondary-stat
  spending. Existing `Stat` enum stays as the target for rune/effect
  bonuses — attributes derive `Stats`' bonus_* fields via a new pure
  `derive_stats(&Attributes) -> Stats` function, layered onto that
  existing plumbing rather than replacing it: `allocate_stat_point` now
  increments the new `Attributes` component, then overwrites `Stats` with
  `derive_stats`'s fresh output, so the two can never drift.
  **`Stats` itself changed shape**, not just its write path:
  `bonus_move_speed`/`bonus_crit_multiplier` were removed entirely (no
  `Attribute` derives either — see the next bullet), and
  `bonus_resistance`/`bonus_damage_percent`/`bonus_attack_speed` were
  added. A new `Attributes` component (replicated, persisted) holds the raw
  invested point counts the character panel displays and spends.
- [x] `Stat` gains an `AttackSpeed` variant (needed so runes can grant
  attack speed directly, alongside Dexterity's own contribution) — summed
  into effective attack speed the same 4-way way crit chance already is:
  active effects + equipment/runes + Dexterity-derived level points, via
  the new `weapon_attack::effective_attack_speed_bonus` (no separate "base"
  term — there's no baseline attack-speed scalar to start from, unlike
  crit chance's `CombatStats.crit_chance`).
- [x] Derived formulas (tuning data, exact curves not final — see
  `MECHANICS.md`'s Open questions):
  - Might → % bonus weapon damage, applied to `base_damage` inside
    `resolve_melee_hit` before crit/resistance.
  - Dexterity → % attack-speed bonus (feeds M8.6's
    `effective_recovery = base_recovery / (1 + bonus)`, recovery only,
    never windup, via the new `weapon_attack::effective_recovery`) + bonus
    crit chance.
  - Vitality → bonus max health + a small flat resistance bonus (stacks
    additively with M8.6's armor resistance and, later, M8.12's Algiz
    rune) — resistance is target-side: `resolve_melee_hit` gained a
    `target_level_stats: Option<&Stats>` parameter (mirroring
    `target_equipment`) so a *defender's* own Vitality investment reduces
    incoming damage, not the attacker's.
  - Intelligence → stored, spendable, **no wired effect until M8.10-12**
    (`derive_stats` has no term for it at all, deliberately — see
    `Stats`'s doc comment).
- [x] **Vitality's `bonus_max_health` interaction with `Health.current`,
  confirmed with the user rather than guessed**: ceiling-only.
  `apply_allocate_stat_point_input` raises `Health.max` by exactly the
  resulting `bonus_max_health` delta on a Vitality spend; `Health.current`
  is left untouched — no free heal from spending a point mid-fight, the
  same "a point spend doesn't retroactively affect anything already in
  flight" behavior the other three attributes already had. A reconnecting
  character with previously-invested Vitality restores the matching
  `Health.max` bonus on spawn (`PLAYER_MAX_HEALTH + stats.bonus_max_health`).
- [x] Migrated `allocate_stat_point`'s tests and the M8 character panel UI
  (labels, spend targets) from direct secondary stats to attributes — the
  panel now shows each attribute's raw invested points plus a short
  player-facing summary of its current derived effect (e.g. "Might: 3
  (+6% weapon damage)"), read from `Stats` rather than recomputed in the
  UI layer.
- [x] Tests: derived-stat formulas (`derive_stats`, `allocate_stat_point`);
  `effective_recovery`/`effective_attack_speed_bonus` (zero-Dexterity case,
  and an extreme-high-value case confirming the divisor never reaches
  zero, per this bullet's original ask); `resolve_melee_hit`'s new Might
  damage-percent and target-side Vitality resistance terms.

**A protocol-crate touch, flagged and confirmed before doing it (per
`CLAUDE.md`'s rule on wire-format changes):** the new `Attributes`
component needed to be replicated (so the character panel can read/spend
it) and persisted (`CharacterSave` gained an `attributes` field, no
`#[serde(default)]` fallback, matching this project's existing
no-backward-compat-shim convention for save-schema changes) —
`protocol::PROTOCOL_ID` bumped from 1 to 2. This makes a pre-M8.7 local
save file fail to load; the user confirmed proceeding and accepted needing
a fresh character.

## M8.8 — Combat feedback: crit flash & critical-health bleeding

Client-only presentation except one small replicated marker for crits — no
`game_core` combat-*resolution* changes beyond exposing crit info and
emitting that marker.

**Implementation status: everything below is built, unit-tested, passes the
full verification loop (build/test/clippy/fmt), and confirmed live** —
server restarted on the new build, client reconnected (existing save still
loaded fine — this milestone didn't touch `CharacterSave`), user confirmed
both combat feel and the new visuals (crit flash, bleeding) working.
Confirmed with the user before touching `protocol` (new `RecentCrit`
component + `PROTOCOL_ID` 2→3 — no save-file impact, ephemeral combat
markers like `Stunned`/`Downed` were never persisted). **Not yet
committed.**

- [x] `resolve_damage` (M4, already shipped and tested) changes its return
  shape to also report whether the hit crit, not just the damage amount —
  now returns a `DamageResult { amount, is_crit }` instead of a bare `f32`.
  A real change to already-tested code; all four of its existing unit
  tests updated (`.amount`/`.is_crit`) alongside two new ones.
- [x] `RecentCrit(f32)` marker component (holds its own remaining seconds,
  `RECENT_CRIT_DURATION = 0.4`, within the 0.3-0.5s range) — replicated
  like `Downed`/`Stunned`, ticked down and removed by a new
  `combat::tick_recent_crit` system (mirrors `tick_attack_timers`'s shape,
  but removes the component at zero rather than clamping, since
  `RecentCrit`'s presence *is* the signal). Inserted from
  `combat::resolve_melee_hit` — the shared hit-resolution helper both
  `attack_system` (enemies) and `weapon_attack::tick_player_attack_phases`
  (players) call — not duplicated into each caller, so a player-dealt crit
  flares exactly like an enemy-dealt one. Both call sites, plus
  `resolve_melee_hit` itself, gained a `Commands` parameter to make the
  insert possible.
- [x] Crit visual: `bevy_lit` intensity-spike-then-decay `PointLight2d`
  burst + a one-shot `bevy_hanabi` particle flare at the target
  (`start_crit_flashes` reacts to `Added<RecentCrit>`; `tick_crit_flashes`
  decays the light and removes the trio), attached directly to the target
  entity so it follows `Transform` automatically — reuses M8.5's torch
  light/particle pattern (`build_crit_flare_effect` mirrors
  `build_torch_spark_effect`, `SpawnerSettings::once` instead of `rate`).
  A placeholder radial-burst shape, not literal "rune glyph" art.
- [x] Critically-low-health bleeding: purely client-side threshold check
  on replicated `Health` via a new `combat::is_critically_low_health` pure
  function (`CRITICALLY_LOW_HEALTH_THRESHOLD = 0.25`) — no new replication.
  Below the threshold, a continuous `bevy_hanabi` blood-mist trickle
  (`bleeding_system`, gated by a client-local `Bleeding` marker so the
  particle effect is only inserted/removed on an actual crossing) plus a
  pulsing dark-red gizmo ring (immediate-mode, reusing
  `status_indicator_system`'s pattern — no bookkeeping needed) persists
  until the ratio rises back above threshold or the entity despawns.
- [x] **Both of the above are gated to a `CombatFeedbackTargets` query
  filter** (`Or<(With<Player>, With<RemotePlayer>, With<Enemy>)>`) — a
  crate breaking shouldn't visually bleed or crit-flash. **Enforced
  client-side, not server-side**: `combat.rs` can't depend on `enemy.rs`'s
  `Enemy` marker without an import cycle (`enemy.rs` already depends on
  `combat.rs`), so `resolve_melee_hit` inserts `RecentCrit` unconditionally
  on any crit, and the client's precise type filter does the excluding —
  see `DECISIONS.md`'s M8.8 entry. No destructible content exists yet
  (M8.9) to actually need excluding either way.
- [x] Player-taken-damage flash/vignette: a full-screen translucent red
  `egui` overlay (`damage_flash_overlay_system`, painted on the background
  layer so it doesn't intercept input), triggered by
  `detect_local_player_damage` diffing the local player's `Health.current`
  frame-to-frame (no replicated "you got hit" event exists or was added)
  and decayed by `tick_damage_flash`. Cheap, high feedback value, entirely
  client-side.
- [ ] Boss health bar: still explicitly **out of scope here**, deferred to
  M9. The crit/bleeding systems are entity-agnostic (`CombatFeedbackTargets`
  covers any `Enemy`, not a specific kind), so a boss gets both for free
  once M9 lands, even before its dedicated top-of-screen bar exists.
- [x] Tests: `game_core` gained threshold-crossing tests
  (`is_critically_low_health` at various ratios including zero-health and
  zero-max-health edge cases) and `RecentCrit` insert/expiry timing
  (`attack_system` inserting/not-inserting it on crit vs. non-crit,
  `tick_recent_crit` counting down and removing at expiry). Bleeding's
  actual visual system lives in `client`, which has no existing unit-test
  convention (nothing else there does), so the pure threshold decision it
  calls is what's tested, in `game_core` where it lives.

**Known gap, flagged not silently dropped: skill-cast crits don't flare.**
`skill::resolve_hit` (power_strike/aoe_burst's damage application) shares
`resolve_damage` and so needed the same signature adaptation, but wasn't
wired to insert `RecentCrit` — that would mean threading `Commands` through
a second call path for a case `ROADMAP.md`'s own wording didn't ask for
this pass. A spell crit not flaring while a melee crit does is a real,
visible inconsistency worth fixing later, not a design decision — see
`DECISIONS.md`'s M8.8 entry.

## M8.9 — Dynamic objects: destructibles & movable puzzle objects

**Implementation status: everything below is built, unit-tested, passes the
full verification loop (build/test/clippy/fmt), and confirmed live.**
Confirmed with the user before touching `protocol` (five new replicated
components — `Destructible`/`DestructibleKind`/`PushableObject`/`Gate`/
`GateOpen` — `PROTOCOL_ID` 3→4, no save-file impact, same as M8.8) and
before authoring a live-testable smoke-test instance beyond what this
section's "no actual puzzle authored yet" wording literally asks for.
**Live-tested in two passes**: the first pass's pushable block drifted
indefinitely once touched (no friction, no damping, and — unlike players/
enemies — nothing re-drives its velocity every tick to mask that), fixed
with `LinearDamping`; that led to a follow-up design ask (movable vs.
immovable destructibles, a weight-driven push feel), built and then
**re-confirmed live** — breaking crates/barrels/pillars, pushing the
weighted block, and the gate opening all working. **Not yet committed.**

### Destructibles
- [x] `content::DestructibleTemplate` (health, loot table, optional
  resistances) — same shape as `EnemyTemplate` minus AI/`XpReward`/attack
  fields, reusing the established "new content type, spawned from a Tiled
  object layer" pattern (interactables, torches).
  **Found via review, fixed before this could ship as a silent bug:**
  `combat::resolve_melee_hit` reads *both* attacker's and target's
  `ActiveEffects` unconditionally (`Query::get_many_mut([attacker,
  target])`) and silently no-ops the whole hit if either is missing it —
  exactly the "stat exists but does nothing" class M8.6's DECISIONS.md
  entry warns about, just for a component presence instead of a formula
  input. `content::spawn_destructible` includes `ActiveEffects::default()`
  for this reason, even though a destructible has no AI to ever apply an
  effect to itself.
- [x] `spawn_destructibles`, reading a new `"destructibles"` Tiled object
  layer, mirroring `spawn_interactables`/`spawn_torch_lights`. A new
  `DestructibleTemplate::movable` field (default `false`) branches spawn
  physics: immovable destructibles (e.g. a stone pillar) are
  `RigidBody::Static` — no ongoing `PhysicsPosition`↔`Position` sync
  needed, only a one-time placement at spawn — while `movable: true`
  destructibles (e.g. a crate) are spawned exactly like a pushable object
  (`RigidBody::Dynamic` + `game_core::PushableObject`, see below), added to
  `server`'s `PhysicsBodies` sync filter that way rather than needing a
  second filter branch. **Confirmed live, not assumed**: the user
  specifically asked for this movable/immovable split and a weight-driven
  push feel after the first pushable-object pass shipped — a real
  gameplay-feel decision surfaced via playtesting, not something
  `MECHANICS.md`/`ROADMAP.md` had already specified.
- [x] No changes to `attack_system`'s nearest-target search — destructibles
  participate in the exact same `With<Health>` targeting as enemies, same
  priority, by design (see `MECHANICS.md`). `death_system` already
  handles the drop/despawn path unmodified.
- [x] Three real `.ron` destructible templates: `crate_common` (movable,
  weight `1.0`, a mixed item/rune/currency loot table), `barrel_currency`
  (immovable, currency-only), and `stone_pillar` (immovable, no loot — added
  specifically to prove the `movable: false` default branch with a real
  content example, matching the user's own illustrative example) — all
  three placed live in `valley.tmx`'s new `"destructibles"` layer, in the
  open bottom field alongside the existing enemy spawns.

### Movable puzzle objects & gates
- [x] Pushable object (`game_core::PushableObject`, a replicated marker): a
  `RigidBody::Dynamic` entity with mass, no `Health`/AI. **Not quite "no
  new physics work" as originally scoped** — confirmed via a real live
  playtest, not assumed: reusing the enemy-push bundle verbatim
  (`Friction::ZERO`, no damping) let the block drift indefinitely once
  touched, because unlike players/enemies nothing re-drives a pushable
  object's velocity every tick to mask a lack of damping. Fixed with a new
  shared `server::pushable_physics(weight) -> (ColliderDensity,
  LinearDamping)` helper — used by both the smoke-test block (`weight:
  1.0`) and any `movable: true` destructible — so a heavier `weight` feels
  harder to budge and settles back to rest almost immediately, while a
  lighter one is easier to nudge with a touch more give, never drifting
  indefinitely at any weight. `server`'s `PhysicsBodies` sync filter
  widened to include `PushableObject` (destructibles that stay immovable,
  and gates, being `RigidBody::Static`, deliberately were not).
- [x] Generic `Unlockable` gate primitive (`game_core::dynamic_object`): an
  entity whose collider toggles between blocking and passable, driven by
  `Vec<UnlockCondition>` where **all conditions must hold simultaneously
  (AND, not OR)** — stated explicitly in the type's doc comment, since
  M8.13 builds against it and the semantics matter for that milestone's
  multi-key gate idea. Starts with one variant, `ObjectInZone { object,
  zone }`, shaped so a `HasKeyItem` variant (M8.13) can be added later
  without restructuring. `Unlockable` itself is deliberately **not**
  replicated — the client only needs to know a gate is currently open, not
  why, so a separate always-present `Gate` marker (for the client to
  identify a gate entity at all) plus `GateOpen` (present only while open,
  replicated like `Downed`/`Stunned`) carry that across the wire instead.
- [x] Trigger-zone check: `unlock_conditions_met`/`update_unlockables`, a
  system comparing a pushable object's `Position` against a defined target
  area (a Tiled rectangle object's world-space bounds, resolved once at
  spawn into a plain `Zone { min_x, max_x, min_y, max_y }`), toggling the
  matching `Unlockable`'s `GateOpen` presence when satisfied.
  **A real crate-boundary constraint surfaced during implementation, not
  guessed around**: `game_core` has no physics-engine dependency
  (`CLAUDE.md`'s crate boundaries), so `update_unlockables` can only
  toggle the game_core-level `GateOpen` marker — it can't touch avian2d's
  `ColliderDisabled` directly. A new `server`-only `sync_gate_collider`
  system bridges the two, the same "physics wiring lives in `server`" role
  `sync_enemy_velocity_to_physics` already plays for enemies. See
  `DECISIONS.md`'s M8.9 entry for the full reasoning.
- [x] Puzzle placement/layout is per-dungeon content (M9's job) — this
  milestone builds the mechanical primitives only. **One exception,
  confirmed with the user**: a single minimal smoke-test instance (one
  pushable block, one target zone, one gate) was placed in `valley.tmx`'s
  new `"puzzle"` layer, clearly commented as a mechanism smoke test (the
  same role M8.5's first test light/occluder played), not real puzzle
  design — `server`'s `spawn_pushables_and_gates` hardcodes matching this
  one fixed-name instance rather than building the generic named-linking
  scheme M9 will actually need for real dungeon puzzles.
- [x] Tests: zone-overlap detection (`Zone::contains`, inclusive
  boundaries), gate state toggling via `update_unlockables` (opens once
  its condition is met, closes again once it stops holding, stays closed
  when its referenced object has despawned), multi-condition AND semantics
  (a gate with two conditions stays closed until *both* hold, not just
  one).

## M8.10 — Rune discovery & learning

**Implementation status: everything below is built, unit-tested, and passes
the full verification loop (build/test/clippy/fmt).** Built autonomously in
a session continued from M8.9's commit/push (no new design round with the
user beyond "start M8.10"); confirmed with the user before touching
`protocol` (`PROTOCOL_ID` 4→5 — four new replicated components, two new
client messages — see `DECISIONS.md`'s M8.10 entry) and before adding
`rand` as a new direct dependency of `server` (already an exact-pinned
workspace dependency used elsewhere, but a new dependency edge for this
specific crate).

**Confirmed live**, across an unusually long live-debugging session (see
`DECISIONS.md` for the full trace) that also surfaced and resolved two
real, unrelated dev-ops gaps along the way (a stale in-memory game
password on server restart, and unsaved progress lost to a hard server
kill — the latter now tracked as its own `M8.10 follow-up` below).
Confirmed pieces, each via a live server+client session with
`tracing::debug!` instrumentation on `interact_or_pickup_system` (kept
permanently, see below): discovery-on-pickup and the resulting
`RuneInventory`/`DiscoveredRunes` state; the `KnownRunes` socket gate
(`crit_shard`, picked up but not yet known, was correctly rejected by
Forging's Socket button; `swift_shard`, granted directly by the runestone,
socketed successfully); the runestone's `grants_rune` direct-grant path;
and the full rune-casting round trip (`RequestRuneCastInput` → offered
`crit_shard` as the sole eligible candidate, correctly excluding the
already-known `swift_shard` → `SelectRuneCastInput` → immediately
socketable afterward).

**One precondition was hand-set, not earned organically, and that's
disclosed rather than hidden**: reaching `UnspentRuneCasts > 0` requires a
level-up, and `spawn_enemies` places exactly one enemy per
`assets/enemies/*.ron` file (two total, 25 XP combined, no respawn system)
— structurally short of the 100 XP `xp_required(1)` needs, regardless of
play skill. Rather than force several server restarts to grind it, the
character's `rune_casts` field was hand-edited directly in its
`saves/default/characters/<id>.ron` save file (while disconnected, per the
same clean-disconnect discipline as any other save edit) to bootstrap a
testable value. This is now documented as standard practice in
`README.md`'s "Live playtest operations" section. The round trip itself
(message plumbing, server resolution, replication, client panel) is fully
confirmed live; reaching that precondition through organic XP gain is not,
and isn't expected to be practical until more enemy content exists.

The initial bug report this session ("no rune in starter loot," "runes not
in inventory") turned out to be user-side confusion (checking the wrong
panel; not realizing `crit_shard`/`swift_shard` *are* the rune ids), not a
real defect — see `DECISIONS.md`.

**A live-debugging decision worth recording**: `interact_or_pickup_system`
gained permanent `tracing::debug!` instrumentation (pickup events + the
direct-grant event) as a result of this detour, deliberately *not* removed
afterward — mirrors `weapon_attack.rs`'s existing precedent of keeping
this kind of debug-gated instrumentation permanently rather than treating
every debugging session's logging as throwaway. See `DECISIONS.md`.

- [x] `DiscoveredRunes` (set, grows on first pickup of a rune type — wired
  into the existing `item::pickup_loot`, alongside the existing
  `RuneInventory` stack increment) and `KnownRunes` (set, mirrors
  `KnownSkills`) components, both replicated and persisted like the rest
  of progression state. New `game_core::rune` module.
- [x] `UnspentRuneCasts`, granted via `grant_xp` on level-up alongside the
  existing stat/skill points — count gated by Intelligence via a new
  `rune::rune_casts_granted(intelligence)` (base 1 + 1 per 5 points
  invested; tuning data, not a settled curve, see `MECHANICS.md`'s Open
  questions). `grant_xp` itself widened to take `&mut UnspentRuneCasts` +
  the caster's current `intelligence: u32`, which meant threading
  `Attributes`/`UnspentRuneCasts` through `combat::AttackerProgress`,
  `weapon_attack::TickingPlayers`, and `skill::SkillCasters` — the same
  "add one more optional field, thread it through every call site" cost
  M8.7/M8.8 already established as routine here.
- [x] `socket_rune` gains a `KnownRunes` membership check — the one new
  gate on otherwise-unchanged, already-tested socket/unsocket code (the
  M8.11/M8.12 combine functions don't exist yet, so there's nothing else
  to gate this pass).
- [x] "Rune casting" panel at the blacksmith (reuses the existing
  `nearest_interactable_with_panel` pattern via a new
  `RUNE_CASTING_PANEL_ID`, added to `blacksmith.ron`'s `opens_panels` — no
  dedicated sejdr NPC yet, nothing else justifies one existing this pass).
  **A new two-message round trip, not a single-button action like every
  other panel so far** — confirmed with the user before implementing, not
  guessed at: `RequestRuneCastInput` spends one `UnspentRuneCasts` and has
  the server sample up to 3 candidates from `DiscoveredRunes − KnownRunes`
  into a new replicated `RuneCastOffer` component (fewer than 3 available
  → offers whatever exists; nothing available → rejected, no point
  spent); `SelectRuneCastInput` confirms one offered candidate into
  `KnownRunes`. The spend happens at request time, not selection time —
  see `DECISIONS.md` for the reasoning and for why the selection step is
  deliberately *not* re-gated by proximity the way the request is.
- [x] New `Interactable` payload: `InteractableDefinition`/
  `content::InteractableTemplate` gain `grants_rune: Option<String>`, for
  direct narrative grants that bypass casting — applied unconditionally
  alongside `effect` in `interact_or_pickup_system`. Naturally claimable
  only once in effect (`KnownRunes`/`DiscoveredRunes` are sets — a repeat
  interaction is a no-op insert), no separate "already claimed" state
  needed. `runestone.ron` gained `grants_rune: Some("swift_shard")`
  alongside its existing crit-chance buff as the real content proof.
- [x] `effective_magnitude = base_magnitude * (1 + intelligence_bonus)`
  wired in at the same point `item::Equipment::stat_bonus` sums rune
  contributions — the primary Intelligence hook, see `MECHANICS.md`. Wired
  generically at the shared `stat_bonus` function itself (a new
  `intelligence_bonus: f32` parameter, sourced from a new `Stats::
  bonus_rune_magnitude` field derived from `Attributes::intelligence`),
  not duplicated per formula — every existing rune-sourced bonus (crit
  chance/multiplier, attack speed, move speed) picks up the scaling for
  free from this one change, matching MECHANICS.md's literal wording that
  this applies wherever `stat_bonus` already sums rune contributions, not
  just a new bespoke calculation.
- [x] Tests: cast sampling excludes already-known runes and rejects when
  either no casts are unspent or nothing is left to offer; the socket gate
  rejects an unknown rune even with stock and currency on hand; direct
  grants add to both `KnownRunes` and `DiscoveredRunes`, and an
  interactable with no `grants_rune` leaves both untouched; `stat_bonus`'s
  magnitude scaling at a representative intelligence bonus; `rune_casts_
  granted`'s curve at zero and higher intelligence.

## M8.10 follow-up — graceful server shutdown save

**Found via this session's live-testing, not a new design ask — flagged
high priority.** The server's persistence model has always been
save-on-disconnect only (see M5's persistence entry): `on_character_
disconnected`, an observer reacting to a client's connection actually
closing, is the *only* place `persistence::save_character_save` is called
after initial character creation — no periodic autosave, no
graceful-shutdown hook. Restarting the server mid-session during this
milestone's live-debugging loop (via `kill <pid>`, a hard `SIGTERM`)
silently discarded every connected player's progress since their last
*clean* disconnect every single time, since a killed process never gives
that observer a chance to run before it dies. Not a bug in any shipped
milestone's game logic — a pre-existing dev-ops gap this session's
unusually long live-debugging loop happened to expose repeatedly (see
`DECISIONS.md`).

- [ ] Install a signal handler (SIGINT/SIGTERM — likely the `ctrlc` or
  `signal-hook` crate; **new dependency, flag before adding** per
  `CLAUDE.md`) that triggers a save for every currently-connected
  character before the process actually exits, reusing the exact same
  `persistence::save_character_save` path `on_character_disconnected`
  already calls rather than duplicating that logic.
- [ ] Confirm live: with a connected client holding unsaved progress
  (picked-up loot, equipped gear, spent currency), kill/restart the
  server, then reconnect and verify that progress survived.

## M8.11 — Repeated-rune combining

- [ ] `RuneTemplate` gains `tier: u32` and `upgrades_into: Option<String>`
  (explicit, content-authored — not formula-derived).
- [ ] Combine action: consume `combine_count` copies (2-9, capped by a
  pure `max_combine_count(intelligence)` function) of the *same* known
  rune from `RuneInventory` → produce 1 copy of its `upgrades_into`
  target, quadratic currency cost per tier (existing `MECHANICS.md` stub).
  Result always shown plainly before confirming — fully deterministic.
- [ ] A first real tiered chain (e.g. `sowilo_t1` → `sowilo_t2`) to prove
  the schema.
- [ ] Tests: combine rejects an unknown rune, rejects insufficient stack
  size, `max_combine_count` curve at representative Intelligence values.

## M8.12 — Bind runes & rune powers

- [ ] New `BindRecipe { inputs: Vec<String>, output: String }` content
  list (order-independent multiset match) — curated, hand-authored
  combinations, not exhaustive coverage of every possible input set.
- [ ] Combine action, bind branch: consume the matching known runes from
  `RuneInventory`, produce the output. **First successful attempt at a
  given recipe reveals it server-wide** (shared discovery, not
  per-character — matches the precedent that rune/item *definitions* are
  universal content); revealed recipes show their result plainly on
  future attempts, same as repeated combining. Unrecognized combinations
  fail with no reveal.
- [ ] `RuneDefinition` extended to optionally carry a flat bonus-damage-of-
  a-`DamageType` and/or a chance-per-hit `EffectDefinition`, alongside the
  existing `Stat` bonus — both fold into the effective-attack computation
  M8.6 already built to read socketed runes. This alone covers Kenaz
  (bonus flame damage) and Isaz (on-hit chill) with no new engine concept,
  just new inputs to formulas that already exist.
- [ ] **Reactive runes need genuinely new plumbing, not reuse**: a new
  check against the *target's* equipped runes inside `attack_system`'s
  damage-application step (today, effects only ever apply from the
  attacker's own effect list) — required for Thurisaz (reactive
  when-hit stun). Call this out explicitly during implementation; it's
  the one piece of this milestone that isn't just wiring existing systems
  together.
- [ ] Starter rune roster (`.ron` content, ~8 to prove breadth): Kenaz,
  Isaz, Thurisaz, Algiz (flat resistance), Sowilo (crit stat, tiered per
  M8.11), Tiwaz (Od-cost-for-damage tradeoff), Ansuz (Od regen), Fehu
  (bonus currency on kill) — see `MECHANICS.md` for each — plus at least
  one authored bind recipe combining two or three of these into a unique
  result.
- [ ] Tests: recipe matching (order-independent), reveal-once-then-shown
  behavior, unknown-combination rejection, reactive-rune trigger-on-being-
  hit (not on landing a hit).

## M8.13 — Stateful inventory: keys & artifacts

- [ ] `content::KeyItemTemplate` (id, display name, flavor text,
  `consumed_on_use: bool`) — loaded into a `KeyItemLibrary`, same shape
  as `ItemLibrary`/`RuneLibrary`.
- [ ] `KeyItems(HashSet<String>)` player component — possession-by-presence,
  same pattern as `KnownRunes`/`KnownSkills`/`DiscoveredRunes`. Replicated,
  and folded into `CharacterSave` alongside the rest of persisted
  progression state.
- [ ] `DroppedLoot::KeyItem(String)` — new branch on the existing enum,
  merged into `KeyItems` on pickup exactly like items/runes/currency
  today. `LootTable`'s `LootKind` gains a matching variant, so a boss (or
  any enemy/destructible from M8.9) can drop one directly.
- [ ] `Unlockable` (M8.9) gains its anticipated second condition variant:
  `UnlockCondition::HasKeyItem(String)`. Interacting with a locked gate
  checks possession against **all** of its conditions (AND semantics,
  already established in M8.9); if the matching template is
  `consumed_on_use`, remove it from `KeyItems` on success. Requiring more
  than one key item at a single gate — including from different party
  members — needs no new engineering, just authoring more than one
  condition.
- [ ] Inventory panel (M8): a small "Key Items" section listing possessed
  keys/artifacts by flavor text — extends the existing panel rather than
  a new screen.
- [ ] A couple of real content examples: one dungeon-scoped consumable key
  + one boss-dropped persistent artifact, to prove both branches of
  `consumed_on_use`.
- [ ] Tests: consumable key removed after successful unlock, artifact
  retained after unlock, multi-condition gate requiring more than one key
  item, gate correctly staying locked when any one condition fails.

## M9 — Objectives & first dungeon content
- [ ] Hand-authored dungeon instance, entered explicitly from the overworld
- [ ] One full objective sequence, tier-1 enemies only
- [ ] Special-character dialog: one-way objective/flavor text, no branching
  — see `MECHANICS.md`. The generic trigger mechanism (`game_core::interact`'s
  `Interactable`) lands in M8 for the blacksmith/runestone case — this
  bullet is about the actual objective-granting *content*, not the
  mechanism, which already exists by the time M9 starts. See `DECISIONS.md`'s
  M8 planning entry.
- [ ] Neutral (unkillable) character type
- [ ] Boss mechanics: phases, enrage timer, arena bounds — see
  `MECHANICS.md`; first boss encounter as part of this dungeon
- [ ] Resurrection-point checkpointing (auto-updates at dungeon entry /
  objectives); full-wipe auto-respawn there — see `MECHANICS.md`'s
  Progression section. Moved here from M5: this needs the dungeon-entry/
  objective triggers this milestone actually builds, which didn't exist
  when M5's XP/persistence work landed.

## M10 — Polish pass
- [ ] Splash/title screen
- [ ] Placeholder audio (CC0 SFX + music) wired in
- [ ] Second co-op playtest end-to-end: two players, overworld + dungeon + loot

---

Beyond M10: broaden enemy tiers (M9's structure repeats), expand skill/item
pools, revisit the "completable vs. infinite" open question from `DESIGN.md`.
