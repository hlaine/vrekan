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
- [ ] Skill acquisition and upgrade, data-driven like enemies/items
- [ ] At least 2-3 skills with distinct mechanical behavior
- [ ] "Öd" resource (regen + combat/action-generated), power attacks that
  consume it — see `MECHANICS.md`

## M7 — Items & forging
- [ ] Item drops, pickup, equip
- [ ] Affix/forging system (the "custom system" from `DESIGN.md`)
- [ ] Loot tables tied to enemy tiers
- [ ] Vendor buy/sell economy, individual currency per player — see
  `MECHANICS.md`
- [ ] Enemy visual-variant data shape (shared base template + swappable
  sprite field) — see `MECHANICS.md`

## M8 — UI: HUD & menus
- [ ] egui HUD: health/resource (öd), cooldowns, minimap, party status,
  downed-state indicator
- [ ] Inventory, skill tree, forging UI, vendor/shop UI — built alongside
  M6/M7, not deferred wholesale to the end
- [ ] Level-up / stat-allocation panel (manual stat points — see
  `MECHANICS.md`)
- [ ] Player skin preset selection; equipped armor/helmet renders visually
  (full per-item outfit changes — see `MECHANICS.md`)

## M9 — Objectives & first dungeon content
- [ ] Hand-authored dungeon instance, entered explicitly from the overworld
- [ ] One full objective sequence, tier-1 enemies only
- [ ] Special-character dialog: one-way objective/flavor text, no branching
  — see `MECHANICS.md`
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
