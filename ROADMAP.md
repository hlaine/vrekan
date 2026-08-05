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
- [ ] Vendor buy/sell economy, individual currency per player — see
  `MECHANICS.md`. Deferred to a part 2 pass: `MECHANICS.md` itself calls
  this "a distinct system" from the loot/forging pipeline above. Currency
  is built once here and shared by both vendors and forging cost (see
  `DECISIONS.md`'s M7 part 2 planning entry): a currency drop is another
  weighted `LootTable`/`LootEntry` entry, not a separate guaranteed-per-
  kill mechanism. Socketing (M7 part 1's `socket_rune`) will gain a
  currency cost; unsocketing stays free/reversible as shipped. Exact
  amounts/scaling deliberately not decided yet.
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

Deferred (see `DECISIONS.md` for why — blocked on weapon-driven combat
stats not existing yet, not a UI-only gap):
- [ ] `TAB`-style toggle-through skill/attack selector, dedicated
  per-attack hotkeys, primary/secondary weapon slots
- [ ] Vendor/shop UI (blocked on M7 part 2's vendor economy, which
  doesn't exist yet)
- [ ] Player skin preset selection; equipped armor/helmet renders visually
  (full per-item outfit changes — see `MECHANICS.md`). Blocked on actual
  art assets, not just code — everything's solid-color placeholder
  sprites today.
- [ ] Minimap, full party status detail

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
